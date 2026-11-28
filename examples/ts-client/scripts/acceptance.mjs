const baseUrl = process.env.IRIS_URL ?? "http://127.0.0.1:9876";

const healthResponse = await fetch(`${baseUrl}/health`);
if (!healthResponse.ok) {
  throw new Error(`health returned ${healthResponse.status}`);
}
const health = await healthResponse.json();

const { listThreads } = await import("../dist/src/usage.js");
const threads = await listThreads(baseUrl, process.env.IRIS_API_TOKEN);
if (!Array.isArray(threads)) {
  throw new Error("listThreads did not return an array");
}

console.log(JSON.stringify({ health, threadCount: threads.length }));
