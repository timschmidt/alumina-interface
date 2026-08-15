import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const cdpPort = process.argv[2] ?? "9224";
const mode = process.argv[3] ?? "single";
if (!["single", "repeat", "recovery", "confirm-recovery"].includes(mode)) {
  throw new Error(
    "cached-job mode must be single, repeat, recovery, or confirm-recovery",
  );
}
const repeat = mode === "repeat";
const recovery = mode === "recovery";
const confirmRecovery = mode === "confirm-recovery";
const repository = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const harnessPath = "/tests/browser/worker-clock-harness.html";
const deviceIds = [
  "414c554d2d53494d3a54494e59424545",
  "414c554d2d53494d3a54494e59424546",
];

const jobIds = repeat ? ["2047934465", "2047934466"] : ["2047934465"];
const cachedJobRequests = jobIds.map((jobId) => {
  const requestText = execFileSync(
    "cargo",
    [
      "run",
      "--quiet",
      "--locked",
      "--offline",
      "-p",
      "alumina-interface",
      "--bin",
      "alumina-cam-fixture",
      "--",
      "--job-id",
      jobId,
      "--device-id",
      deviceIds[0],
      "--device-id",
      deviceIds[1],
    ],
    {
      cwd: repository,
      encoding: "utf8",
      env: { ...process.env, NO_COLOR: "false" },
      stdio: ["ignore", "pipe", "inherit"],
    },
  );
  return JSON.parse(requestText);
});
const cachedJobRequest = repeat ? cachedJobRequests : cachedJobRequests[0];
const expectation = repeat
  ? "cached-job-repeat"
  : confirmRecovery
    ? "cached-job-confirm-recovery"
    : recovery
      ? "cached-job-recovery"
      : "cached-job";

const pages = await fetch(`http://127.0.0.1:${cdpPort}/json`).then((response) =>
  response.json(),
);
const page = pages.find((candidate) => candidate.type === "page");
if (!page) throw new Error("no Chromium page is available through CDP");

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

await cdp("Page.navigate", {
  url: `http://127.0.0.1:8097${harnessPath}?expect=${expectation}`,
});

const installationDeadline = Date.now() + 30000;
let installed = false;
while (Date.now() < installationDeadline) {
  try {
    const ready = await cdp("Runtime.evaluate", {
      expression: "typeof window.installCachedJobRequest === 'function'",
      returnByValue: true,
    });
    if (ready.result.value === true) {
      const installation = await cdp("Runtime.evaluate", {
        expression: `window.installCachedJobRequest(${JSON.stringify(cachedJobRequest)}); true`,
        returnByValue: true,
      });
      installed = installation.result.value === true;
      if (installed) break;
    }
  } catch {
    // Navigation may replace the execution context between polls.
  }
  await new Promise((resolve) => setTimeout(resolve, 100));
}
if (!installed) {
  socket.close();
  throw new Error("cached-job request could not be installed in the harness");
}

// The in-page workflow owns the 180-second protocol deadline. Retain enough
// polling grace to observe and report that exact terminal diagnostic.
const deadline = Date.now() + 195000;
let result;
let lastInspection;
let priorProgressState;
while (Date.now() < deadline) {
  try {
    const evaluation = await cdp("Runtime.evaluate", {
      expression:
        "({ status: document.querySelector('#result')?.dataset.status, text: document.querySelector('#result')?.textContent, inspection: window.inspectCachedJobHarness?.() })",
      returnByValue: true,
    });
    const value = evaluation.result.value;
    if (value?.inspection !== undefined) {
      lastInspection = value.inspection;
      const latest = lastInspection.latest_cached_job_snapshot;
      const progressState = JSON.stringify([
        lastInspection.request_index,
        latest?.job_id,
        latest?.phase,
        latest?.consecutive_failures,
        latest?.participants.map((participant) => [
          participant.cache_artifact,
          participant.cache_phase,
          participant.schedule_phase,
        ]),
      ]);
      if (latest !== undefined && progressState !== priorProgressState) {
        console.error(
          `[cached-job] ${JSON.stringify({
            snapshot_count: lastInspection.snapshot_count,
            request_index: lastInspection.request_index,
            job_id: latest.job_id,
            phase: latest.phase,
            consecutive_failures: latest.consecutive_failures,
            last_error: latest.last_error,
            participants: latest.participants.map((participant) => ({
              connection_id: participant.connection_id,
              cache_artifact: participant.cache_artifact,
              cache_phase: participant.cache_phase,
              accepted_bytes: participant.accepted_bytes,
              total_bytes: participant.total_bytes,
              schedule_phase: participant.schedule_phase,
            })),
          })}`,
        );
        priorProgressState = progressState;
      }
    }
    if (value?.status === "passed" || value?.status === "failed") {
      result = { status: value.status, detail: JSON.parse(value.text) };
      break;
    }
  } catch {
    // Keep polling while a transient execution context is replaced.
  }
  await new Promise((resolve) => setTimeout(resolve, 200));
}
socket.close();
if (!result) {
  throw new Error(
    `cached-job harness did not reach a terminal state: ${JSON.stringify(lastInspection)}`,
  );
}
if (result.status === "passed") {
  const detail = result.detail;
  const compactRun = (run) => {
    const phaseSequence = [];
    for (const transition of run.cached_job_transitions ?? []) {
      if (phaseSequence.at(-1) !== transition.phase) {
        phaseSequence.push(transition.phase);
      }
    }
    return {
      job_id: run.job_id ?? run.latest_cached_job_snapshot?.job_id,
      cached_job_snapshot_count: run.cached_job_snapshot_count,
      phase_sequence: phaseSequence,
      latest_cached_job_snapshot: run.latest_cached_job_snapshot,
      cached_job_failure_observations:
        run.cached_job_failure_observations ?? [],
      cached_job_recovered: run.cached_job_recovered ?? false,
    };
  };
  const completedCachedJobs = (detail.completed_cached_jobs ?? []).map(compactRun);
  result.detail = repeat
    ? {
        expectation: detail.expectation,
        completed_cached_jobs: completedCachedJobs,
        device_snapshots: detail.device_snapshots,
      }
    : {
        expectation: detail.expectation,
        ...compactRun(detail),
        device_snapshots: detail.device_snapshots,
      };
}
console.log(JSON.stringify(result, null, 2));
if (result.status !== "passed") process.exitCode = 1;
