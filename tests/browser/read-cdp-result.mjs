const port = process.argv[2] ?? "9224";
const scenario = process.argv[3];
const expectation = process.argv[4] ?? "qualified";
const harnessPath = "/tests/browser/worker-clock-harness.html";
const pages = await fetch(`http://127.0.0.1:${port}/json`).then((response) =>
  response.json(),
);
const page = pages.find(
  (candidate) =>
    candidate.type === "page" && new URL(candidate.url).pathname === harnessPath,
);
if (!page) throw new Error("worker clock harness page is not open");

const socket = new WebSocket(page.webSocketDebuggerUrl);
const pending = new Map();
let nextId = 1;
await new Promise((resolve, reject) => {
  socket.onopen = resolve;
  socket.onerror = () => reject(new Error("CDP websocket failed"));
});
socket.onmessage = (message) => {
  const response = JSON.parse(message.data);
  const request = pending.get(response.id);
  if (!request) return;
  pending.delete(response.id);
  if (response.error) request.reject(new Error(response.error.message));
  else request.resolve(response.result);
};

const cdp = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });

if (scenario) {
  const query = new URLSearchParams({ scenario, expect: expectation });
  await cdp("Page.navigate", {
    url: `http://127.0.0.1:8097${harnessPath}?${query}`,
  });
}

const deadline = Date.now() + 25000;
let result;
while (Date.now() < deadline) {
  try {
    const evaluation = await cdp("Runtime.evaluate", {
      expression:
        "({ status: document.querySelector('#result')?.dataset.status, text: document.querySelector('#result')?.textContent })",
      returnByValue: true,
    });
    const value = evaluation.result.value;
    if (value?.status === "passed" || value?.status === "failed") {
      result = { status: value.status, detail: JSON.parse(value.text) };
      break;
    }
  } catch {
    // Navigation may replace the execution context between polls.
  }
  await new Promise((resolve) => setTimeout(resolve, 200));
}
socket.close();
if (!result) throw new Error("worker clock harness did not reach a terminal state");
console.log(JSON.stringify(result, null, 2));
if (result.status !== "passed") process.exitCode = 1;
