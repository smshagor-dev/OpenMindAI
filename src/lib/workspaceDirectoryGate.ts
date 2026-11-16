export type WorkspaceDirectoryRequest = Readonly<{
  generation: number;
  projectId: string;
  rootId: string | null;
  path: string;
}>;

// Keeps async listings and file actions tied to their originating project/root/path.
export class WorkspaceDirectoryGate {
  private generation = 0;
  private selection: { projectId: string; rootId: string | null } | null = null;
  private displayed: WorkspaceDirectoryRequest | null = null;

  select(projectId: string, rootId: string | null): void {
    this.generation += 1;
    this.selection = { projectId, rootId };
    this.displayed = null;
  }

  begin(projectId: string, rootId: string | null, path: string): WorkspaceDirectoryRequest | null {
    if (!this.isSelected(projectId, rootId)) return null;
    this.displayed = null;
    return { generation: ++this.generation, projectId, rootId, path };
  }

  isCurrent(request: WorkspaceDirectoryRequest): boolean {
    return request.generation === this.generation &&
      this.isSelected(request.projectId, request.rootId);
  }

  accept(request: WorkspaceDirectoryRequest): boolean {
    if (!this.isCurrent(request)) return false;
    this.displayed = request;
    return true;
  }

  isDisplayed(projectId: string, rootId: string | null, path: string): boolean {
    return this.displayed !== null && this.isCurrent(this.displayed) &&
      this.displayed.projectId === projectId && this.displayed.rootId === rootId &&
      this.displayed.path === path;
  }

  private isSelected(projectId: string, rootId: string | null): boolean {
    return this.selection?.projectId === projectId && this.selection?.rootId === rootId;
  }
}
