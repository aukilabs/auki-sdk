import { stat } from "node:fs/promises";

export async function assertFreshPkgWeb({ artifact, sources, buildCommand }) {
  let artifactStat;
  try {
    artifactStat = await stat(artifact);
  } catch (err) {
    throw new Error(
      `pkg-web artifact is missing: ${artifact}\nRun: ${buildCommand}`,
      { cause: err },
    );
  }

  const staleSources = [];
  for (const source of sources) {
    const sourceStat = await stat(source);
    if (sourceStat.mtimeMs > artifactStat.mtimeMs) {
      staleSources.push(source);
    }
  }

  if (staleSources.length > 0) {
    throw new Error(
      [
        "pkg-web artifacts are stale; the browser smoke would run old wasm/js.",
        `Newest source is newer than ${artifact}.`,
        "Stale sources:",
        ...staleSources.map((source) => `- ${source}`),
        `Run: ${buildCommand}`,
      ].join("\n"),
    );
  }
}
