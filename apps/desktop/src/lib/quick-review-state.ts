export function diffRangeFromSourceLabel(sourceLabel: string | null | undefined): string {
  if (!sourceLabel) return "";
  if (!sourceLabel.startsWith("cli:")) return sourceLabel;

  const firstSeparator = sourceLabel.indexOf(":", 4);
  if (firstSeparator < 0) return sourceLabel;
  return sourceLabel.slice(firstSeparator + 1);
}

export function repoPrefKey(repoPath: string): string {
  const bytes = new TextEncoder().encode(repoPath);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}
