export type ParkSettings = {
  sdk_contents_root: string;
  default_sdk_contents_root: string;
  settings_path: string;
};

const STORAGE_KEY = "auki.overwatch.settings.sdkContentsRoot.v1";
const defaultSettings: ParkSettings = {
  sdk_contents_root: "",
  default_sdk_contents_root: "",
  settings_path: "browser-local-storage",
};

export async function fetchSettings(): Promise<ParkSettings> {
  return {
    ...defaultSettings,
    sdk_contents_root: localStorage.getItem(STORAGE_KEY) ?? "",
  };
}

export async function saveSdkContentsRoot(
  sdkContentsRoot: string,
): Promise<ParkSettings> {
  localStorage.setItem(STORAGE_KEY, sdkContentsRoot);
  throw new Error("Browser Overwatch does not have a filesystem SDK contents root.");
}
