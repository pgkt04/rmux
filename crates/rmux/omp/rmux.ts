import type { Subprocess } from "bun";
import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type { TUI } from "@oh-my-pi/pi-tui";

export default function (pi: ExtensionAPI) {
	let ui: TUI | undefined;
	let observer: Subprocess<"pipe", "pipe", "pipe"> | undefined;
	let generation = 0;
	let disabled = false;
	let disposeReconciliation: (() => void) | undefined;
	let reprobeing = false;
	let manualReprobe: ((view: "native" | "ansi") => void) | undefined;

	function reprobe(view: "native" | "ansi") {
		if (!ui) throw new Error("rmux terminal UI is unavailable");
		const previous = process.env.PI_TUI_NATIVE;
		reprobeing = true;
		try {
			ui.stop();
			process.env.PI_TUI_NATIVE = disabled ? "0" : "1";
			ui.start();
			ui.requestRender(true);
		} finally {
			reprobeing = false;
			if (previous === undefined) delete process.env.PI_TUI_NATIVE;
			else process.env.PI_TUI_NATIVE = previous;
		}
		pi.logger.debug("rmux terminal renderer reprobed", { view });
	}

	pi.on("session_start", async (_event, ctx) => {
		generation++;
		const current = generation;
		disposeReconciliation?.();
		disposeReconciliation = undefined;
		observer?.kill();
		observer = undefined;
		ui = undefined;
		if (!ctx.hasUI || !process.env.RMUX || !/^%\d+$/.test(process.env.RMUX_PANE ?? "")) return;
		disabled = process.env.PI_TUI_NATIVE === "0";
		ctx.ui.setWidget("rmux-terminal", tui => {
			ui = tui;
			return { render: () => [], invalidate() {} };
		});
		ctx.ui.setWidget("rmux-terminal", undefined);
		if (disabled) return;

		const socket = process.env.RMUX.split(",")[0];
		const pane = process.env.RMUX_PANE!;
		const session = await pi.exec("rmux", ["-N", "-S", socket, "display-message", "-p", "-t", pane, "#{session_id}"], { timeout: 5000 });
		if (current !== generation) return;
		if (session.code !== 0 || !/^\$\d+$/.test(session.stdout.trim())) {
			throw new Error(`rmux observer could not find this pane's session: ${session.stderr.trim()}`);
		}
		const environment = { ...process.env };
		delete environment.RMUX;
		delete environment.RMUX_PANE;
		const child = Bun.spawn([
			"rmux", "-N", "-S", socket, "-C", "attach-session", "-r", "-E",
			"-f", "no-output,ignore-size", "-t", session.stdout.trim(),
		], { env: environment, stdin: "pipe", stdout: "pipe", stderr: "pipe" });
		observer = child;
		child.stdin.write(`refresh-client -B 'rmux-omp:${pane}:#{pane_tsp_view}'\n`);
		child.stdin.flush();
		const tui = ui!;
		let latestView: string | undefined;
		let desiredView: "native" | "ansi" | undefined;
		let retry: Timer | undefined;
		let stopped = false;
		let attempts = 0;
		let warned = false;
		let editorPaused = false;
		let settleUntil = 0;
		let retryAfter = 0;
		const backoff = [100, 250, 1000];
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
			if (stopped || current !== generation || ui !== tui || reprobeing) return;
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
		const removeStartListener = tui.addStartListener(() => {
			if (reprobeing || stopped || current !== generation) return;
			cancelReconciliation();
			resetAttempts();
			scheduleReconciliation();
		});
		const runManualReprobe = (view: "native" | "ansi") => {
			if (stopped || current !== generation || ui !== tui) return;
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
		manualReprobe = runManualReprobe;
		const dispose = () => {
			stopped = true;
			latestView = undefined;
			cancelReconciliation();
			removeStartListener();
			if (manualReprobe === runManualReprobe) manualReprobe = undefined;
		};
		disposeReconciliation = dispose;

		const errors = new Response(child.stderr).text();
		void (async () => {
			const reader = child.stdout.getReader();
			const decoder = new TextDecoder();
			let pending = "";
			try {
				while (current === generation) {
					const { value, done } = await reader.read();
					if (done) break;
					pending += decoder.decode(value, { stream: true });
					let newline: number;
					while ((newline = pending.indexOf("\n")) !== -1) {
						const line = pending.slice(0, newline);
						pending = pending.slice(newline + 1);
						if (!line.startsWith("%subscription-changed rmux-omp ")) continue;
						const view = line.slice(line.lastIndexOf(" : ") + 3).trim();
						if (!["native", "ansi", "detached", "pending"].includes(view)) {
							throw new Error("rmux server does not support pane_tsp_view; restart it with the updated binary");
						}
						if (view !== latestView) {
							cancelReconciliation();
							if ((view === "native" || view === "ansi") && view !== desiredView) {
								desiredView = view;
								resetAttempts();
							}
							latestView = view;
							settleUntil = view === "ansi" ? Date.now() + 150 : 0;
							scheduleReconciliation();
						}
					}
				}
			} finally {
				dispose();
				if (disposeReconciliation === dispose) disposeReconciliation = undefined;
				reader.releaseLock();
			}
			const code = await child.exited;
			const error = await errors;
			if (current === generation) {
				observer = undefined;
				ctx.ui.notify(`rmux terminal observer stopped (${code})${error.trim() ? `: ${error.trim()}` : ""}; use /terminal-reprobe`, "warning");
			}
		})().catch(error => {
			if (current !== generation) return;
			child.kill();
			dispose();
			if (disposeReconciliation === dispose) disposeReconciliation = undefined;
			observer = undefined;
			pi.logger.error("rmux terminal observer failed", { error: String(error) });
			ctx.ui.notify("rmux terminal observer failed; use /terminal-reprobe", "error");
		});
	});

	pi.registerCommand("terminal-reprobe", {
		description: "Re-negotiate this rmux pane's native or text UI without restarting omp",
		handler: async (_args, ctx) => {
			if (!ui || !process.env.RMUX || !process.env.RMUX_PANE) {
				ctx.ui.notify("/terminal-reprobe requires an interactive rmux pane", "warning");
				return;
			}
			if (disabled) {
				reprobe("ansi");
				return;
			}
			const current = generation;
			const result = await pi.exec("rmux", ["-N", "display-message", "-p", "-t", process.env.RMUX_PANE, "#{pane_tsp_view}"], { timeout: 5000 });
			const view = result.stdout.trim();
			if (current !== generation || !ui) return;
			if (result.code !== 0 || (view !== "native" && view !== "ansi")) {
				ctx.ui.notify(`rmux terminal view is ${view || "unavailable"}; retry after attaching a terminal`, "warning");
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
		ui = undefined;
	});
}
