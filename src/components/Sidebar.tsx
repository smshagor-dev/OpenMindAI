import { useEffect, useRef, useState } from "react";
import {
  Archive,
  Database,
  Globe,
  FolderKanban,
  FolderOpen,
  LibraryBig,
  MoreHorizontal,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  SquarePen,
  TableProperties,
  Wrench,
} from "lucide-react";
import type { Conversation, UserProfile } from "../types";
import { ChatHistoryList } from "./ChatHistoryList";
import packageJson from "../../package.json";
import { sidebarText } from "../lib/sidebarLocale";

const MIN_SIDEBAR_WIDTH = 260;
const MAX_SIDEBAR_WIDTH = 440;

export function Sidebar(props: {
  collapsed: boolean;
  language?: string;
  onToggleCollapsed: () => void;
  conversations: Conversation[];
  activeId: string | null;
  view:
    | "chat"
    | "work"
    | "settings"
    | "tools"
    | "projects"
    | "research-studio"
    | "3d-studio"
    | "web-search";
  userProfile: UserProfile | null;
  width: number;
  onWidthChange: (width: number) => void;
  onNewChat: () => void;
  onOpenSearch: () => void;
  onOpenArchived: () => void;
  onOpenConversation: (id: string) => void;
  onRename: (conversation: Conversation) => void;
  onTogglePin: (conversation: Conversation) => void;
  onArchive: (id: string) => void;
  onDelete: (id: string) => void;
  onDuplicate: (conversation: Conversation) => void;
  onOpenLibrary: () => void;
  onOpenModels: () => void;
  onOpenDatasets: () => void;
  onOpenTools: () => void;
  onOpenProjects: () => void;
  onOpenStudio: (section: "research-studio" | "3d-studio" | "web-search") => void;
  onOpenSettings: (section?: string) => void;
}) {
  const t = (key: Parameters<typeof sidebarText>[1]) => sidebarText(props.language, key);
  const displayName =
    props.userProfile?.preferredName || props.userProfile?.fullName || "Local User";
  const [profileMenuOpen, setProfileMenuOpen] = useState(false);
  const [resizing, setResizing] = useState(false);
  const profileMenuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!profileMenuOpen) return;
    const onClickOutside = (event: MouseEvent) => {
      if (profileMenuRef.current && !profileMenuRef.current.contains(event.target as Node)) {
        setProfileMenuOpen(false);
      }
    };
    window.addEventListener("mousedown", onClickOutside);
    return () => window.removeEventListener("mousedown", onClickOutside);
  }, [profileMenuOpen]);

  useEffect(() => {
    if (!resizing) return;
    const onMouseMove = (event: MouseEvent) => {
      const next = Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, event.clientX));
      props.onWidthChange(next);
    };
    const onMouseUp = () => setResizing(false);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, [resizing, props]);

  return (
    <aside
      className={props.collapsed ? "sidebar collapsed" : resizing ? "sidebar resizing" : "sidebar"}
      style={props.collapsed ? undefined : { width: props.width }}
    >
      <div className="sidebar-header">
        <div className="brand">
          <img className="brand-mark" src="/icon.png" alt="OpenMindAI" />
          {!props.collapsed ? (
            <div>
              <strong>OpenMindAI</strong>
              <small>Your AI. Your Models. Your Way.</small>
            </div>
          ) : null}
        </div>
        {!props.collapsed ? (
          <div className="sidebar-header-actions">
            <button
              className="icon-button"
              title={t("search")}
              onClick={props.onOpenSearch}
            >
              <Search size={16} />
            </button>
            <button
              className="icon-button sidebar-collapse-toggle"
              title={t("collapse")}
              onClick={props.onToggleCollapsed}
            >
              <PanelLeftClose size={16} />
            </button>
          </div>
        ) : (
          <button
            className="icon-button sidebar-collapse-toggle"
            title={t("expand")}
            onClick={props.onToggleCollapsed}
          >
            <PanelLeftOpen size={16} />
          </button>
        )}
      </div>

      <button className="primary-command" onClick={props.onNewChat} title={t("newChat")}>
        <SquarePen size={18} /> {!props.collapsed ? t("newChat") : null}
      </button>

      <nav className="sidebar-nav">
        <button className="nav-button" onClick={props.onOpenArchived} title="Archived chats">
          <Archive size={18} /> {!props.collapsed ? "Archived chats" : null}
        </button>
        <button className="nav-button" onClick={props.onOpenLibrary} title={t("library")}>
          <LibraryBig size={18} /> {!props.collapsed ? t("library") : null}
        </button>
        <button className="nav-button" onClick={props.onOpenModels} title={t("models")}>
          <Database size={18} /> {!props.collapsed ? t("models") : null}
        </button>
        <button className="nav-button" onClick={props.onOpenDatasets} title={t("datasets")}>
          <TableProperties size={18} /> {!props.collapsed ? t("datasets") : null}
        </button>
        <button
          className={props.view === "tools" ? "nav-button active" : "nav-button"}
          onClick={props.onOpenTools}
          title={t("tools")}
        >
          <Wrench size={18} /> {!props.collapsed ? t("tools") : null}
        </button>
        <button
          className={props.view === "research-studio" ? "nav-button active" : "nav-button"}
          onClick={() => props.onOpenStudio("research-studio")}
          title={t("research")}
        >
          <TableProperties size={18} /> {!props.collapsed ? t("research") : null}
        </button>
        <button
          className={props.view === "3d-studio" ? "nav-button active" : "nav-button"}
          onClick={() => props.onOpenStudio("3d-studio")}
          title={t("studio3d")}
        >
          <Wrench size={18} /> {!props.collapsed ? t("studio3d") : null}
        </button>
        <button
          className={props.view === "web-search" ? "nav-button active" : "nav-button"}
          onClick={() => props.onOpenStudio("web-search")}
          title={t("webSearch")}
        >
          <Globe size={18} /> {!props.collapsed ? t("webSearch") : null}
        </button>
        <button className="nav-button" onClick={props.onOpenLibrary} title={t("files")}>
          <FolderOpen size={18} /> {!props.collapsed ? t("files") : null}
        </button>
        <button
          className={props.view === "projects" ? "nav-button active" : "nav-button"}
          onClick={props.onOpenProjects}
          title={t("projects")}
        >
          <FolderKanban size={18} /> {!props.collapsed ? t("projects") : null}
        </button>
        <button
          className={props.view === "settings" ? "nav-button active" : "nav-button"}
          onClick={() => props.onOpenSettings()}
          title={t("more")}
        >
          <MoreHorizontal size={18} /> {!props.collapsed ? t("more") : null}
        </button>
      </nav>

      {props.view === "3d-studio" || props.view === "research-studio" ? (
        // Portal target for the active studio's list (scenes / research projects).
        // Always mounted for studio views so its node identity stays stable
        // across sidebar collapse/expand; hidden via CSS when collapsed.
        <div id="studio-sidebar-slot" className="sidebar-studio-slot" />
      ) : (
        <ChatHistoryList
          conversations={props.conversations}
          activeId={props.activeId}
          collapsed={props.collapsed}
          onOpen={props.onOpenConversation}
          onRename={props.onRename}
          onTogglePin={props.onTogglePin}
          onArchive={props.onArchive}
          onDelete={props.onDelete}
          onDuplicate={props.onDuplicate}
        />
      )}

      <div className="sidebar-footer" ref={profileMenuRef}>
        <div className="sidebar-profile">
          <button
            className="profile-trigger"
            title={displayName}
            onClick={() =>
              props.collapsed ? props.onOpenSettings() : setProfileMenuOpen((value) => !value)
            }
          >
            <span className="profile-avatar">
              {props.userProfile?.avatarDataUrl ? (
                <img src={props.userProfile.avatarDataUrl} alt="" />
              ) : (
                displayName.charAt(0).toUpperCase() || "?"
              )}
            </span>
            {!props.collapsed ? (
              <span className="profile-text">
                <span className="profile-name">{displayName}</span>
                <span className="profile-role">v{packageJson.version}</span>
              </span>
            ) : null}
          </button>
          {!props.collapsed ? (
            <button
              className="profile-menu-trigger"
              title={t("profileOptions")}
              onClick={() => setProfileMenuOpen((value) => !value)}
            >
              <MoreHorizontal size={16} />
            </button>
          ) : null}
        </div>
        {profileMenuOpen ? (
          <div className="profile-menu" role="menu">
            <button
              role="menuitem"
              onClick={() => {
                props.onOpenSettings("general");
                setProfileMenuOpen(false);
              }}
            >
              {t("settings")}
            </button>
            <button
              role="menuitem"
              onClick={() => {
                props.onOpenSettings("personalization");
                setProfileMenuOpen(false);
              }}
            >
              {t("personalization")}
            </button>
            <button
              role="menuitem"
              onClick={() => {
                props.onOpenSettings("appearance");
                setProfileMenuOpen(false);
              }}
            >
              {t("appearance")}
            </button>
            <button
              role="menuitem"
              onClick={() => {
                props.onOpenSettings("about");
                setProfileMenuOpen(false);
              }}
            >
              {t("about")}
            </button>
          </div>
        ) : null}
      </div>
      {!props.collapsed ? (
        <div
          className={resizing ? "sidebar-resize-handle active" : "sidebar-resize-handle"}
          onMouseDown={(event) => {
            event.preventDefault();
            setResizing(true);
          }}
        />
      ) : null}
    </aside>
  );
}
