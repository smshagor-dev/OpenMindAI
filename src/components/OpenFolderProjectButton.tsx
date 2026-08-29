import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, LoaderCircle } from "lucide-react";
import { useState } from "react";
import { api } from "../api";
import { localWorkspaceApi } from "../lib/localWorkspace";
import { formatError } from "../lib/format";
import type { Conversation, Project } from "../types";

export function OpenFolderProjectButton(props: {
  disabled?: boolean;
  onCreateProjectChat: (project: Project) => Promise<Conversation>;
  onCreated: (project: Project, conversation: Conversation) => Promise<void> | void;
  onError: (message: string) => void;
}) {
  const [busy, setBusy] = useState(false);

  const openFolderProject = async () => {
    if (busy || props.disabled) return;
    setBusy(true);
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Open a local folder as an OpenMindAI project",
      });
      if (!selected || Array.isArray(selected)) return;

      const projectName = folderName(selected) || "Local project";
      const project = await api.createProject(projectName);
      await localWorkspaceApi.attachFolder(project.id, selected);

      const grantFullAccess = window.confirm(
        `OpenMindAI attached:\n${selected}\n\nEnable Full PC + Terminal for this project?\n\nChoose OK to let the Project Agent run local build/test/install/git/terminal commands and use absolute paths with your current OS-user permissions. Choose Cancel to keep the agent restricted to the attached folder's file tools.`,
      );
      if (grantFullAccess) {
        await localWorkspaceApi.setFullAccess(project.id, true, true);
      }

      const conversation = await props.onCreateProjectChat(project);
      await api.linkProjectConversation(project.id, conversation.id);
      await props.onCreated(project, conversation);
    } catch (caught) {
      props.onError(formatError(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <button
      type="button"
      className="ghost-button project-open-folder-button"
      disabled={busy || props.disabled}
      onClick={() => void openFolderProject()}
      title="Choose a PC folder, create a project, attach the folder, and start a linked agent chat"
    >
      {busy ? <LoaderCircle className="spin" size={15} /> : <FolderOpen size={15} />}
      {busy ? "Opening…" : "Open folder as project"}
    </button>
  );
}

function folderName(path: string) {
  const cleaned = path.replace(/[\\/]+$/, "");
  return cleaned.split(/[\\/]/).filter(Boolean).pop()?.trim() ?? "";
}
