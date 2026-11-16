/** Installed badges are provisional until the backend exposes a validated disk inventory. */
export function initialInstalledDatasets(): Set<string> {
  // Never read historical localStorage IDs: the portable root may have moved,
  // the files may have been deleted, or the install may have been interrupted.
  return new Set<string>();
}

export function recordCompletedDataset(
  current: ReadonlySet<string>,
  state: string | null | undefined,
  datasetId: string | null | undefined,
): Set<string> {
  if (state !== "completed" || !datasetId?.trim() || current.has(datasetId)) {
    return current as Set<string>;
  }
  const next = new Set(current);
  next.add(datasetId);
  return next;
}
