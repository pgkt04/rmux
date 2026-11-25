import type { Subprocess } from "bun";
import { fstatSync, statSync } from "node:fs";
import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

type View = "native" | "ansi";

/** omp through 19.1: the TUI a widget factory receives. Stopping and starting it repeats omp's hello. */
interface LegacyTui {
	stop(): void;
	start(): void;
	requestRender(force?: boolean): void;
	addStartListener(listener: () => void): () => void;
	readonly nativeRendering: boolean;
	readonly terminal: { readonly tspProbePending: boolean };
}

/** omp from 19.2: the renderer is chosen once per process, so only a new process repeats the hello. */
interface RestartingOmp {
	native(): boolean;
	/** omp's own `/restart`; absent when this omp does not expose it. */
	restart?: () => Promise<void>;
}

const VIEW_NAMES: Record<View, string> = { native: "native", ansi: "text" };
/** Carries the view a restart was meant to reach into the process it starts. */
const RESTARTED = "RMUX_OMP_RESTARTED";
/** A restart that left omp native anyway is not repeated within this long. */
const RESTART_LOOP_MS = 10_000;
/** How long the pane must need text before omp restarts into it: another pane sharing the window stays. */
const SHARED_SETTLE_MS = 300;
/** The same, for causes rmux does not name (a menu, a prompt, a plain viewer): most end on their own. */
const UNNAMED_SETTLE_MS = 5000;
const BUSY_POLL_MS = 1000;

function isLegacyTui(value: unknown): value is LegacyTui {
	const tui = value as Partial<LegacyTui> | undefined;
	return typeof tui?.stop === "function" && typeof tui.start === "function" && typeof tui.addStartListener === "function";
}

export default function (pi: ExtensionAPI) {
	let legacy: LegacyTui | undefined;
	let restarting: RestartingOmp | undefined;
	let observer: Subprocess<"pipe", "pipe", "pipe"> | undefined;
	let generation = 0;
	let disabled = false;
	let disposeReconciliation: (() => void) | undefined;
	let reprobeing = false;
	let manualReprobe: ((view: View) => void) | undefined;

	function reprobe(view: View) {
		if (!legacy) throw new Error("rmux terminal UI is unavailable");
		const previous = process.env.PI_TUI_NATIVE;
		reprobeing = true;
		try {
			legacy.stop();
			process.env.PI_TUI_NATIVE = disabled ? "0" : "1";
			legacy.start();
			legacy.requestRender(true);
		} finally {
			reprobeing = false;
			if (previous === undefined) delete process.env.PI_TUI_NATIVE;
			else process.env.PI_TUI_NATIVE = previous;
		}
		pi.logger.debug("rmux terminal renderer reprobed", { view });
	}

	/**
	 * omp 19.2 gives extensions no way to renegotiate, but its interactive mode has `restart()`
	 * (the `/restart` command). `ctx.ui.refreshTranscript` calls a method of that mode, so
	 * standing in for the method once hands over the instance.
	 */
	async function restartingOmp(ctx: ExtensionContext): Promise<RestartingOmp | undefined> {
		try {
			// Imported here, not statically: what these host modules export depends on the omp that
			// loaded the plugin, and a missing export must leave the plugin loaded.
			const { nativeRendering } = (await import("@oh-my-pi/pi-tui")) as { nativeRendering?: () => boolean };
			if (typeof nativeRendering !== "function") return undefined;
			const host: RestartingOmp = { native: () => nativeRendering() };
			const { InteractiveMode } = (await import("@oh-my-pi/pi-coding-agent")) as {
				InteractiveMode?: { prototype: Record<string, unknown> };
			};
			const prototype = InteractiveMode?.prototype;
			const refresh = prototype?.refreshTranscriptPresentation;
			const ui = ctx.ui as { refreshTranscript?: (ids: string[]) => void };
			if (!prototype || typeof refresh !== "function" || typeof prototype.restart !== "function" || !ui.refreshTranscript) {
				return host;
			}
			let mode: { restart(): Promise<void> } | undefined;
			prototype.refreshTranscriptPresentation = function (this: { restart(): Promise<void> }) {
				mode = this;
			};
			try {
				ui.refreshTranscript([]);
			} finally {
				prototype.refreshTranscriptPresentation = refresh;
			}
			if (mode) {
				const owner = mode;
				host.restart = () => owner.restart();
			}
			return host;
		} catch (error) {
			pi.logger.debug("rmux terminal cannot reach omp's renderer state", { error: String(error) });
			return undefined;
		}
	}

	function busy(ctx: ExtensionContext): boolean {
		// A paused stdin means an external program (an editor) owns the tty.
		return !ctx.isIdle() || ctx.hasPendingMessages() || process.stdin.isPaused();
	}

	/** Replace this omp with a new one in the same pane; rmux answers its hello for the view the pane has then. */
	function restartInto(host: RestartingOmp, view: View, ctx: ExtensionContext) {
		if (!host.restart) {
			ctx.ui.notify(`rmux: run /restart to switch omp to the ${VIEW_NAMES[view]} view`, "info");
			return;
		}
		process.env[RESTARTED] = `${view}:${Date.now()}`;
		pi.logger.debug("rmux terminal restarting omp", { view });
		// A restart that happens never returns; one omp refused leaves this process as it was.
		host.restart().then(
			() => {
				delete process.env[RESTARTED];
			},
			error => {
				delete process.env[RESTARTED];
				pi.logger.error("rmux terminal could not restart omp", { error: String(error) });
				ctx.ui.notify(`rmux: omp did not restart; run /restart to switch to the ${VIEW_NAMES[view]} view`, "warning");
			},
		);
	}

	function terminalDevices(): number[] {
		const devices: number[] = [];
		for (const fd of [0, 1, 2]) {
			try {
				const stat = fstatSync(fd);
				if (stat.isCharacterDevice()) devices.push(stat.rdev);
			} catch {}
		}
		return devices;
	}

	async function ownPaneSession(socket: string, pane: string): Promise<string | undefined> {
		const identity = await pi.exec("rmux", ["-N", "-S", socket, "display-message", "-p", "-t", pane, "#{session_id} #{pane_tty}"], { timeout: 5000 });
		const match = /^(\$\d+) (\/\S+)$/.exec(identity.stdout.trim());
		if (identity.code !== 0 || !match) {
			pi.logger.debug("rmux pane unavailable; plugin inactive", { socket, pane, error: identity.stderr.trim() });
			return undefined;
		}
		// macOS ps reports "stdin" as the tty of processes in some panes, so the pane's
		// device number is compared with omp's own terminal descriptors instead.
		const paneTty = match[2];
		let paneDevice: number;
		try {
			const stat = statSync(paneTty);
			if (!stat.isCharacterDevice()) throw new Error("not a terminal device");
			paneDevice = stat.rdev;
		} catch (error) {
			pi.logger.debug("rmux pane tty unavailable; plugin inactive", { socket, pane, paneTty, error: String(error) });
			return undefined;
		}
		const ownDevices = terminalDevices();
		if (!ownDevices.includes(paneDevice)) {
			pi.logger.debug("rmux pane is not omp's terminal; plugin inactive", { socket, pane, paneTty, paneDevice, ownDevices });
			return undefined;
		}
		return match[1];
	}

	pi.on("session_start", async (_event, ctx) => {
		generation++;
		const current = generation;
		disposeReconciliation?.();
		disposeReconciliation = undefined;
		observer?.kill();
		observer = undefined;
		legacy = undefined;
		restarting = undefined;
		if (!ctx.hasUI || !process.env.RMUX || !/^%\d+$/.test(process.env.RMUX_PANE ?? "")) return;
		const socket = process.env.RMUX.split(",")[0];
		const pane = process.env.RMUX_PANE!;
		const session = await ownPaneSession(socket, pane);
		if (current !== generation || !session) return;
		disabled = process.env.PI_TUI_NATIVE === "0";
		ctx.ui.setWidget("rmux-terminal", (tui: unknown) => {
			if (isLegacyTui(tui)) legacy = tui;
			return { render: () => [], invalidate() {} };
		});
		ctx.ui.setWidget("rmux-terminal", undefined);
		const tui = legacy;
		// The restart this process came from, if any: read once, and not inherited by what omp runs.
		const restarted = /^(native|ansi):(\d+)$/.exec(process.env[RESTARTED] ?? "");
		delete process.env[RESTARTED];
		let host: RestartingOmp | undefined;
		if (!tui) {
			host = await restartingOmp(ctx);
			if (current !== generation) return;
			if (!host) {
				ctx.ui.notify("rmux-terminal does not support this omp; run: rmux update, then rmux update-plugin", "warning");
				return;
			}
			restarting = host;
		}
		if (disabled) return;

		const environment = { ...process.env };
		delete environment.RMUX;
		delete environment.RMUX_PANE;
		let latestView: string | undefined;
		let desiredView: View | undefined;
		let retry: Timer | undefined;
		let stopped = false;
		let attempts = 0;
		let warned = false;
		let editorPaused = false;
		let settleUntil = 0;
		let retryAfter = 0;
		// Only a restarting omp reads these: why and since when the pane wants its view.
		let viewSince = 0;
		let inMode = false;
		let shared = false;
		let hinted = false;
		const backoff = [100, 250, 1000];
		// Another client's `attach-session -d` detaches the observer too, so it reattaches.
		const reconnectBackoff = [100, 1000, 5000];
		let reconnects = 0;
		let reconnect: Timer | undefined;
		function cancelReconciliation() {
			if (retry !== undefined) ctx.clearTimer(retry);
			retry = undefined;
		}
		function resetAttempts() {
			attempts = 0;
			warned = false;
			retryAfter = 0;
		}
		function scheduleReconciliation(delay = 0) {
			if (stopped || current !== generation || retry !== undefined) return;
			retry = ctx.setTimeout(() => {
				retry = undefined;
				reconcile();
			}, delay);
		}
		function reconcile() {
			if (tui) reconcileLegacy(tui);
			else if (host) reconcileRestarting(host);
		}
		function reconcileLegacy(tui: LegacyTui) {
			if (stopped || current !== generation || legacy !== tui || reprobeing) return;
			if (latestView !== "native" && latestView !== "ansi") return;
			if (!tui.terminal.tspProbePending && tui.nativeRendering === (latestView === "native")) return;
			if (process.stdin.isPaused()) {
				editorPaused = true;
				scheduleReconciliation(50);
				return;
			}
			if (editorPaused) {
				editorPaused = false;
				resetAttempts();
			}
			if (tui.terminal.tspProbePending) {
				scheduleReconciliation(50);
				return;
			}
			if (tui.nativeRendering === (latestView === "native")) return;
			const delay = Math.max(settleUntil, retryAfter) - Date.now();
			if (delay > 0) {
				scheduleReconciliation(delay);
				return;
			}
			if (attempts === backoff.length) {
				if (!warned) {
					warned = true;
					ctx.ui.notify(`rmux terminal negotiation did not reach ${latestView}; use /terminal-reprobe`, "warning");
				}
				return;
			}
			retryAfter = Date.now() + backoff[attempts++];
			reprobe(latestView);
			scheduleReconciliation(50);
		}
		function reconcileRestarting(host: RestartingOmp) {
			if (stopped || current !== generation || restarting !== host) return;
			if (latestView !== "native" && latestView !== "ansi") return;
			const native = host.native();
			if (native === (latestView === "native")) return;
			if (!native) {
				// Text in a pane that could be native still shows omp: restarting is the user's call.
				if (!hinted) {
					hinted = true;
					ctx.ui.notify("rmux: this pane can show omp natively; /terminal-reprobe restarts omp into that view", "info");
				}
				return;
			}
			// Native in a pane that needs text: rmux draws its note over omp until omp paints rows.
			// A pane mode (copy mode) ends by itself and returns the native view.
			if (inMode) return;
			const delay = viewSince + (shared ? SHARED_SETTLE_MS : UNNAMED_SETTLE_MS) - Date.now();
			if (delay > 0) {
				scheduleReconciliation(delay);
				return;
			}
			if (restarted?.[1] === "ansi" && Date.now() - Number(restarted[2]) < RESTART_LOOP_MS) {
				if (!warned) {
					warned = true;
					ctx.ui.notify("rmux: omp restarted for the text view and is native again; zoom this pane, or use /terminal-reprobe", "warning");
				}
				return;
			}
			// A restart ends the turn in flight, so it waits for the agent.
			if (busy(ctx)) {
				scheduleReconciliation(BUSY_POLL_MS);
				return;
			}
			restartInto(host, "ansi", ctx);
		}
		const removeStartListener =
			tui?.addStartListener(() => {
				if (reprobeing || stopped || current !== generation) return;
				cancelReconciliation();
				resetAttempts();
				scheduleReconciliation();
			}) ?? (() => {});
		const runManualReprobe = (view: View) => {
			if (stopped || current !== generation || legacy !== tui || !tui) return;
			cancelReconciliation();
			resetAttempts();
			latestView = view;
			desiredView = view;
			settleUntil = 0;
			attempts = 1;
			retryAfter = Date.now() + backoff[0];
			reprobe(view);
			scheduleReconciliation(50);
		};
		if (tui) manualReprobe = runManualReprobe;
		const dispose = () => {
			stopped = true;
			latestView = undefined;
			cancelReconciliation();
			if (reconnect !== undefined) ctx.clearTimer(reconnect);
			reconnect = undefined;
			removeStartListener();
			if (manualReprobe === runManualReprobe) manualReprobe = undefined;
		};
		disposeReconciliation = dispose;

		function observe(target: string) {
			const child = Bun.spawn([
				"rmux", "-N", "-S", socket, "-C", "attach-session", "-r", "-E",
				"-f", "no-output,ignore-size", "-t", target,
			], { env: environment, stdin: "pipe", stdout: "pipe", stderr: "pipe" });
			observer = child;
			// The view first: a server without the later variables still reports it.
			child.stdin.write(`refresh-client -B 'rmux-omp:${pane}:#{pane_tsp_view} #{pane_in_mode} #{window_zoomed_flag} #{window_panes}'\n`);
			child.stdin.flush();
			const errors = new Response(child.stderr).text();
			void (async () => {
				const reader = child.stdout.getReader();
				const decoder = new TextDecoder();
				let pending = "";
				try {
					while (!stopped && current === generation) {
						const { value, done } = await reader.read();
						if (done) break;
						pending += decoder.decode(value, { stream: true });
						let newline: number;
						while ((newline = pending.indexOf("\n")) !== -1) {
							const line = pending.slice(0, newline);
							pending = pending.slice(newline + 1);
							if (!line.startsWith("%subscription-changed rmux-omp ")) continue;
							const [view, modes, zoomed, panes] = line.slice(line.lastIndexOf(" : ") + 3).trim().split(" ");
							if (!["native", "ansi", "detached", "pending"].includes(view)) {
								throw new Error("rmux server does not support pane_tsp_view; restart it with the updated binary");
							}
							reconnects = 0;
							const wasInMode = inMode;
							const wasShared = shared;
							inMode = Number(modes) > 0;
							shared = Number(panes) > 1 && zoomed !== "1";
							if (view !== latestView) {
								cancelReconciliation();
								if (view === "native" || view === "ansi") {
									// A new desired view or a new attach, back from pending/detached,
									// earns a fresh budget: the viewer is not the one that exhausted it.
									if (view !== desiredView || (latestView !== "native" && latestView !== "ansi")) {
										resetAttempts();
									}
									desiredView = view;
								}
								latestView = view;
								viewSince = Date.now();
								settleUntil = view === "ansi" ? Date.now() + 150 : 0;
								scheduleReconciliation();
							} else if (host && (inMode !== wasInMode || shared !== wasShared)) {
								// The view is the same but its cause changed: a restarting omp decides again.
								cancelReconciliation();
								scheduleReconciliation();
							}
						}
					}
				} finally {
					reader.releaseLock();
				}
				const code = await child.exited;
				const error = (await errors).trim();
				if (stopped || current !== generation || observer !== child) return;
				observerStopped(code, error);
			})().catch(error => {
				if (stopped || current !== generation || observer !== child) return;
				child.kill();
				dispose();
				if (disposeReconciliation === dispose) disposeReconciliation = undefined;
				observer = undefined;
				pi.logger.error("rmux terminal observer failed", { error: String(error) });
				ctx.ui.notify("rmux terminal observer failed; use /terminal-reprobe", "error");
			});
		}

		function observerStopped(code: number | null, error: string) {
			observer = undefined;
			// The view may change while no observer watches it, so the next report always reconciles.
			latestView = undefined;
			cancelReconciliation();
			if (reconnects === reconnectBackoff.length) {
				dispose();
				if (disposeReconciliation === dispose) disposeReconciliation = undefined;
				ctx.ui.notify(`rmux terminal observer stopped (${code})${error ? `: ${error}` : ""}; use /terminal-reprobe`, "warning");
				return;
			}
			pi.logger.debug("rmux terminal observer stopped; reattaching", { code, error, attempt: reconnects + 1 });
			reconnect = ctx.setTimeout(() => {
				reconnect = undefined;
				void reattach();
			}, reconnectBackoff[reconnects++]);
		}

		async function reattach() {
			let target: string | undefined;
			let error = "omp's pane is gone from the rmux server";
			try {
				// The pane may have moved to another session.
				target = await ownPaneSession(socket, pane);
			} catch (cause) {
				error = String(cause);
			}
			if (stopped || current !== generation) return;
			if (target) observe(target);
			else observerStopped(null, error);
		}

		observe(session);
	});

	pi.registerCommand("terminal-reprobe", {
		description: "Re-negotiate this rmux pane's native or text UI (omp 19.2 and later restart to do it)",
		handler: async (_args, ctx) => {
			const host = restarting;
			if ((!legacy && !host) || !process.env.RMUX || !process.env.RMUX_PANE) {
				ctx.ui.notify("/terminal-reprobe requires an interactive rmux pane", "warning");
				return;
			}
			if (disabled) {
				if (legacy) reprobe("ansi");
				else ctx.ui.notify("PI_TUI_NATIVE=0 keeps omp on the text view", "info");
				return;
			}
			const current = generation;
			const result = await pi.exec("rmux", ["-N", "-S", process.env.RMUX.split(",")[0], "display-message", "-p", "-t", process.env.RMUX_PANE, "#{pane_tsp_view}"], { timeout: 5000 });
			const view = result.stdout.trim();
			if (current !== generation || (!legacy && restarting !== host)) return;
			if (result.code !== 0 || (view !== "native" && view !== "ansi")) {
				ctx.ui.notify(`rmux terminal view is ${view || "unavailable"}; retry after attaching a terminal`, "warning");
				return;
			}
			if (host) {
				if (host.native() === (view === "native")) {
					ctx.ui.notify(`omp already shows the ${VIEW_NAMES[view]} view this pane wants`, "info");
				} else if (busy(ctx)) {
					ctx.ui.notify("omp restarts to switch views, which ends the turn in flight; retry once it is done", "warning");
				} else {
					// After this handler returns: omp's teardown should not wait on its own command.
					ctx.setTimeout(() => restartInto(host, view, ctx), 0);
				}
				return;
			}
			if (manualReprobe) manualReprobe(view);
			else reprobe(view);
		},
	});

	pi.on("session_shutdown", () => {
		generation++;
		disposeReconciliation?.();
		disposeReconciliation = undefined;
		observer?.kill();
		observer = undefined;
		legacy = undefined;
		restarting = undefined;
	});
}
