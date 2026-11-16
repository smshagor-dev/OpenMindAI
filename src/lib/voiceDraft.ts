export interface VoiceDraftTarget {
  isCurrent: () => boolean;
  append: (transcript: string) => boolean;
}
