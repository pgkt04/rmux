// Read-only comparison against omp's independent TypeScript applier.
const reference = process.env.RMUX_OMP_APPLIER;
if (!reference) throw new Error("RMUX_OMP_APPLIER is required");
// The external read-only reference checkout is selected by the test environment.
const { TspDocument } = await import(reference);
const input = await Bun.stdin.text();
const { surface, frames } = JSON.parse(input);
const document = new TspDocument(surface);
const states = frames.map((frame: unknown) => {
 const errors = document.applyFrame(frame);
 return { tree: document.snapshot(), errors: errors.map((e: {op: number}) => e.op), focus: document.focus, suspended: document.suspended, settled: [...document.settled].sort() };
});
console.log(JSON.stringify(states));
