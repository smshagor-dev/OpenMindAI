/** Centralized confirmation for navigation that would replace an unsaved editor buffer. */
export function confirmWorkspaceDiscard(
  dirty: boolean,
  action: string,
  confirm: (message: string) => boolean = (message) => window.confirm(message),
): boolean {
  // Never invoke a modal for clean editors. A declined confirmation must
  // leave the caller's active root, path and editor state untouched.
  return !dirty || confirm(`Discard unsaved editor changes and ${action}?`);
}
