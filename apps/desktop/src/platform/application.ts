import { invoke } from "@tauri-apps/api/core";

export interface ApplicationInfo {
  name: string;
  version: string;
}

function isApplicationInfo(value: unknown): value is ApplicationInfo {
  return (
    typeof value === "object" &&
    value !== null &&
    "name" in value &&
    typeof value.name === "string" &&
    "version" in value &&
    typeof value.version === "string"
  );
}

export async function getApplicationInfo(): Promise<ApplicationInfo> {
  const response: unknown = await invoke("get_application_info");

  if (!isApplicationInfo(response)) {
    throw new TypeError("Tule received an invalid application-info response.");
  }

  return response;
}
