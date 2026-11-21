const reportBootstrapFailure = (reason) => {
  const detail = reason instanceof Error ? reason.message : String(reason);
  self.postMessage(
    JSON.stringify({
      schema_version: 1,
      event: {
        kind: "fatal",
        message: `control worker bootstrap failed: ${detail.slice(0, 256)}`,
      },
    }),
  );
};

self.addEventListener("unhandledrejection", (event) => {
  event.preventDefault();
  reportBootstrapFailure(event.reason);
});

try {
  const module = await import("./alumina-interface.js");
  await module.default();
  module.start_control_worker();
} catch (error) {
  reportBootstrapFailure(error);
}
