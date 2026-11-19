/** Do not let a misleading extension or MIME override a classified media handler. */
export function shouldReadAttachmentAsText(kind: string): boolean {
  return kind === "text";
}
