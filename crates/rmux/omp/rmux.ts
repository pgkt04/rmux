import type { Subprocess } from "bun";
import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type { TUI } from "@oh-my-pi/pi-tui";

export default function (pi: ExtensionAPI) {
	let ui: TUI | undefined;
	let observer: Subprocess<"pipe", "pipe", "pipe"> | undefined;
	let generation = 0;
	let disabled = false;

	function reprobe(view: "native" | "ansi") {
		if (!ui) throw new Error("rmux terminal UI is unavailable");
		const previous = process.env.PI_TUI_NATIVE;
		ui.stop();
		try {
			process.env.PI_TUI_NATIVE = disabled ? "0" : "1";
			ui.start();
			ui.requestRender(true);
		} finally {
			if (previous === undefined) delete process.env.PI_TUI_NATIVE;
			else process.env.PI_TUI_NATIVE = previous;
		}
		pi.logger.debug("rmux terminal renderer reprobed", { view });
	}

	pi.on("session_start", async (_event, ctx) => {
		generation++;
		const current = generation;
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
		if (session.code !== 0 || !/^\$\d+$/.test(session.stdout.trim())) {
			throw new Error(`rmux observer could not find this pane's session: ${session.stderr.trim()}`);
		}
		if (current !== generation) return;
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

		const errors = new Response(child.stderr).text();
		void (async () => {
			const reader = child.stdout.getReader();
			const decoder = new TextDecoder();
			let pending = "";
			let latestView: string | undefined;
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
						latestView = view;
						if (view !== "native" && view !== "ansi") continue;
						ctx.setTimeout(() => {
							if (current !== generation || latestView !== view || !ui || ui.nativeRendering === (view === "native")) return;
							reprobe(view);
						}, 0);
					}
				}
			} finally {
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
			const result = await pi.exec("rmux", ["-N", "display-message", "-p", "-t", process.env.RMUX_PANE, "#{pane_tsp_view}"], { timeout: 5000 });
			const view = result.stdout.trim();
			if (result.code !== 0 || (view !== "native" && view !== "ansi")) {
				ctx.ui.notify(`rmux terminal view is ${view || "unavailable"}; retry after attaching a terminal`, "warning");
				return;
			}
			reprobe(view);
		},
	});

	pi.on("session_shutdown", () => {
		generation++;
		observer?.kill();
		observer = undefined;
		ui = undefined;
	});
}
