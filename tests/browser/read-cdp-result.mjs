const port = process.argv[2] ?? "9224";
const scenario = process.argv[3];
const expectation = process.argv[4] ?? "qualified";
const compact = process.argv[5] === "compact";
const peek = process.argv[5] === "peek";
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
socket.onclose = () => {
  for (const request of pending.values()) {
    request.reject(new Error("CDP websocket closed"));
  }
  pending.clear();
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

if (peek) {
  const evaluation = await cdp("Runtime.evaluate", {
    expression: "window.inspectCachedJobHarness?.()",
    returnByValue: true,
  });
  socket.close();
  console.log(JSON.stringify(evaluation.result.value, null, 2));
  process.exit(0);
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
if (compact) {
  const eventCounts = Object.create(null);
  for (const kind of result.detail.event_kinds ?? []) {
    eventCounts[kind] = (eventCounts[kind] ?? 0) + 1;
  }
  const compactJobSnapshot = (snapshot) => ({
    job_id: snapshot.job_id,
    phase: snapshot.phase,
    execution_mode: snapshot.execution_mode,
    target_ui_ns: snapshot.target_ui_ns,
    consecutive_failures: snapshot.consecutive_failures,
    last_error: snapshot.last_error,
    participants: snapshot.participants.map((participant) => ({
      connection_id: participant.connection_id,
      generation: participant.generation,
      cache_artifact: participant.cache_artifact,
      cache_phase: participant.cache_phase,
      accepted_bytes: participant.accepted_bytes,
      total_bytes: participant.total_bytes,
      next_chunk: participant.next_chunk,
      schedule_phase: participant.schedule_phase,
      local_start_cycle: participant.local_start_cycle,
    })),
  });
  const compactDeviceSnapshot = (snapshot) => ({
    connection_id: snapshot.connection_id,
    generation: snapshot.generation,
    phase: snapshot.phase,
    boot_id: snapshot.boot_id,
    device_identity: snapshot.device_identity,
    capability_phase: snapshot.capability_phase,
    capability_identity: snapshot.capability_identity,
    configuration_availability: snapshot.configuration_availability,
    configuration: snapshot.configuration,
    consecutive_failures: snapshot.consecutive_failures,
    last_error: snapshot.last_error,
  });
  const jobTransitions = result.detail.cached_job_transitions ?? [];
  let priorJobState;
  for (const snapshot of result.detail.cached_job_snapshots ?? []) {
    const compactSnapshot = compactJobSnapshot(snapshot);
    const state = JSON.stringify({
      phase: compactSnapshot.phase,
      participants: compactSnapshot.participants.map((participant) => [
        participant.cache_artifact,
        participant.cache_phase,
        participant.schedule_phase,
      ]),
    });
    if (state !== priorJobState) jobTransitions.push(compactSnapshot);
    priorJobState = state;
  }
  const latestJobSnapshot =
    result.detail.latest_cached_job_snapshot ??
    result.detail.cached_job_snapshots?.at(-1);
  const deviceSnapshots =
    result.detail.device_snapshots ??
    (result.detail.snapshot === undefined ? [] : [result.detail.snapshot]);
  result.detail = {
    expectation: result.detail.expectation,
    error: result.detail.error,
    event_counts: eventCounts,
    cached_job_snapshot_count:
      result.detail.cached_job_snapshot_count ??
      result.detail.cached_job_snapshots?.length ??
      0,
    cached_job_transitions: jobTransitions,
    last_cached_job_snapshot:
      latestJobSnapshot === undefined
        ? undefined
        : compactJobSnapshot(latestJobSnapshot),
    device_snapshots: deviceSnapshots.map(compactDeviceSnapshot),
  };
}
console.log(JSON.stringify(result, null, 2));
if (result.status !== "passed") process.exitCode = 1;
