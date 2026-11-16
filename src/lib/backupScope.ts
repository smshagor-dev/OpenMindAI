/** Maintenance snapshots are SQLite-only. Never claim these include portable media. */
export const DATABASE_BACKUP_NOTICE =
  "Database-only snapshot: chat records and settings are backed up, but uploaded media, generated files, datasets, and installed models are NOT included.";

export const DATABASE_BACKUP_RESTORE_GUIDE =
  "A .db snapshot cannot restore a complete workspace on its own. For full workspace preservation, close OpenMindAI and separately copy the entire portable storage root, including data/media, data/media-index, generated, and datasets. Do not replace the active database while OpenMindAI is running.";

export function backupScopeLabel(scope: string | null | undefined): string {
  return scope === "databaseOnly" ? "Database only" : "Backup scope unverified";
}
