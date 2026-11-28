import {
  IrisClient,
  type ListThreadsResult,
} from "../generated/ts-client/index.js";

/**
 * The generated surface is the only SDK code a consumer needs to call Iris.
 * The caller supplies the deployment URL/token; no runtime package is needed.
 */
export async function listThreads(
  baseUrl: string,
  token?: string,
): Promise<ListThreadsResult> {
  const client = new IrisClient({ baseUrl, token });
  return client.listThreads({ limit: 10 });
}
