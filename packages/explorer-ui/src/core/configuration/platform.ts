export function isMacOs(): boolean {
  return globalThis.navigator?.platform?.startsWith("Mac") ?? false;
}

export function normalizeWorkspaceSeparators(path: string): string {
  return isMacOs() ? path : path.replaceAll("\\", "/");
}
