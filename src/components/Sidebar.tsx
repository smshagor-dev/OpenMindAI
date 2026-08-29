import { useEffect, useRef, useState } from "react";
import {
  Database,
  FolderKanban,
  FolderOpen,
  LibraryBig,
  MoreHorizontal,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  SquarePen,
  Wrench,
} from "lucide-react";
import type { Conversation, UserProfile } from "../types";
import { api } from "../api";
import { ChatHistoryList } from "./ChatHistoryList";
import packageJson from "../../package.json";

const MIN_SIDEBAR_WIDTH = 260;
const MAX_SIDEBAR_WIDTH = 440;

export function Sidebar(props: {
  collapsed: boolean;
  onToggleCollapsed: () => void;
  conversations: Conversation[];
  activeId: string | null;
  view: "chat" | "work" | "settings" | "tools" | "projects";
  userProfile: UserProfile | null;
  width: number;
  onWidthChange: (width: number) => void;
  onNewChat: () => void;
  onOpenSearch: () => void;
  onOpenConversation: (id: string) => void;
  onRename: (conversation: Conversation) => void;
  onTogglePin: (conversation: Conversation) => void;
  onArchive: (id: string) => void;
  onDelete: (id: string) => void;
  onDuplicate: (conversation: Conversation) => void;
  onOpenLibrary: () => void;
  onOpenModels: () => void;
  onOpenTools: () => void;
  onOpenProjects: () => void;
  onOpenSettings: (section?: string) => void;
}) {
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
              title="Search chats (Ctrl+K)"
              onClick={props.onOpenSearch}
            >
              <Search size={16} />
            </button>
            <button
              className="icon-button sidebar-collapse-toggle"
              title="Collapse sidebar"
              onClick={props.onToggleCollapsed}
            >
              <PanelLeftClose size={16} />
            </button>
          </div>
        ) : (
          <button
            className="icon-button sidebar-collapse-toggle"
            title="Expand sidebar"
            onClick={props.onToggleCollapsed}
          >
            <PanelLeftOpen size={16} />
          </button>
        )}
      </div>

      <button className="primary-command" onClick={props.onNewChat} title="New chat">
        <SquarePen size={18} /> {!props.collapsed ? "New Chat" : null}
      </button>

      <nav className="sidebar-nav">
        <button className="nav-button" onClick={props.onOpenLibrary} title="Library">
          <LibraryBig size={18} /> {!props.collapsed ? "Library" : null}
        </button>
        <button className="nav-button" onClick={props.onOpenModels} title="Models">
          <Database size={18} /> {!props.collapsed ? "Models" : null}
        </button>
        <button
          className={props.view === "tools" ? "nav-button active" : "nav-button"}
          onClick={props.onOpenTools}
          title="Tools"
        >
          <Wrench size={18} /> {!props.collapsed ? "Tools" : null}
        </button>
        <button className="nav-button" onClick={props.onOpenLibrary} title="Files & Artifacts">
          <FolderOpen size={18} /> {!props.collapsed ? "Files & Artifacts" : null}
        </button>
        <button
          className={props.view === "projects" ? "nav-button active" : "nav-button"}
          onClick={props.onOpenProjects}
          title="Projects"
        >
          <FolderKanban size={18} /> {!props.collapsed ? "Projects" : null}
        </button>
        <button
          className={props.view === "settings" ? "nav-button active" : "nav-button"}
          onClick={() => props.onOpenSettings()}
          title="More"
        >
          <MoreHorizontal size={18} /> {!props.collapsed ? "More" : null}
        </button>
      </nav>

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
              title="Profile options"
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
              Settings
            </button>
            <button
              role="menuitem"
              onClick={() => {
                props.onOpenSettings("personalization");
                setProfileMenuOpen(false);
              }}
            >
              Personalization
            </button>
            <button
              role="menuitem"
              onClick={() => {
                props.onOpenSettings("appearance");
                setProfileMenuOpen(false);
              }}
            >
              Appearance
            </button>
            <button
              role="menuitem"
              onClick={() => {
                props.onOpenSettings("about");
                setProfileMenuOpen(false);
              }}
            >
              About
            </button>
          </div>
        ) : null}
        {!props.collapsed ? (
          <div className="sidebar-copyright">
            <p>
              Copyright 2026{" "}
              <a
                href="https://smshagor.com"
                onClick={(event) => {
                  event.preventDefault();
                  void api.openExternalUrl("https://smshagor.com");
                }}
              >
                Md Shahanur Islam Shagor
              </a>
              . All rights reserved.
            </p>
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
