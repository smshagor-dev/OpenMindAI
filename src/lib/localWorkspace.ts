import { invoke } from "@tauri-apps/api/core";

const isTauri = "__TAURI_INTERNALS__" in window;

export interface ProjectWorkspaceRoot {
  id: string;
  path: string;
  label: string;
  exists: boolean;
  writable: boolean;
  createdAt: string;
}

export interface ProjectLocalAccessStatus {
  projectId: string;
  fullPcAccess: boolean;
  terminalEnabled: boolean;
  roots: ProjectWorkspaceRoot[];
}

export interface WorkspaceEntry {
  name: string;
  path: string;
  relativePath: string;
  kind: "file" | "directory" | "symlink" | "other";
  sizeBytes: number | null;
  modifiedAt: string | null;
  hidden: boolean;
}

export interface WorkspaceFileContent {
  path: string;
  content: string;
  sizeBytes: number;
}

export interface TerminalCommandResult {
  command: string;
  cwd: string;
  exitCode: number;
  stdout: string;
  stderr: string;
  durationMs: number;
  timedOut: boolean;
  truncated: boolean;
}

function desktopInvoke<T>(command: string, args: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    return Promise.reject(new Error("Local workspace access requires the OpenMindAI desktop app."));
  }
  return invoke<T>(command, args);
}

export const localWorkspaceApi = {
  status: (projectId: string) =>
    desktopInvoke<ProjectLocalAccessStatus>("project_local_access_status", { projectId }),
  attachFolder: (projectId: string, path: string) =>
    desktopInvoke<ProjectLocalAccessStatus>("attach_project_workspace_folder", { projectId, path }),
  detachFolder: (projectId: string, rootId: string) =>
    desktopInvoke<ProjectLocalAccessStatus>("detach_project_workspace_folder", { projectId, rootId }),
  setFullAccess: (projectId: string, enabled: boolean, approved: boolean) =>
    desktopInvoke<ProjectLocalAccessStatus>("set_project_full_local_access", {
      projectId,
      enabled,
      approved,
    }),
  listDirectory: (projectId: string, rootId: string | null, path: string) =>
    desktopInvoke<WorkspaceEntry[]>("list_project_workspace_directory", {
      projectId,
      rootId,
      path,
    }),
  readFile: (projectId: string, rootId: string | null, path: string) =>
    desktopInvoke<WorkspaceFileContent>("read_project_workspace_file", {
      projectId,
      rootId,
      path,
    }),
  writeFile: (
    projectId: string,
    rootId: string | null,
    path: string,
    content: string,
    approved = true,
  ) =>
    desktopInvoke<{ path: string; kind: string }>("write_project_workspace_file", {
      projectId,
      rootId,
      path,
      content,
      approved,
    }),
  createDirectory: (
    projectId: string,
    rootId: string | null,
    path: string,
    approved = true,
  ) =>
    desktopInvoke<{ path: string; kind: string }>("create_project_workspace_directory", {
      projectId,
      rootId,
      path,
      approved,
    }),
  movePath: (
    projectId: string,
    rootId: string | null,
    sourcePath: string,
    targetPath: string,
    approved = true,
  ) =>
    desktopInvoke<{ path: string; kind: string }>("move_project_workspace_path", {
      projectId,
      rootId,
      sourcePath,
      targetPath,
      approved,
    }),
  deletePath: (
    projectId: string,
    rootId: string | null,
    path: string,
    approved = true,
  ) =>
    desktopInvoke<void>("delete_project_workspace_path", {
      projectId,
      rootId,
      path,
      approved,
    }),
  runTerminal: (
    projectId: string,
    rootId: string | null,
    cwd: string,
    command: string,
    approved = true,
  ) =>
    desktopInvoke<TerminalCommandResult>("run_project_terminal_command", {
      projectId,
      rootId,
      cwd,
      command,
      approved,
    }),
};
