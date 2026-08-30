import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  Bot,
  CircleHelp,
  Database,
  FileBox,
  FolderKanban,
  FolderOpen,
  Info,
  LibraryBig,
  Menu,
  MessageSquareText,
  MoreHorizontal,
  PanelLeftClose,
  PanelLeftOpen,
  PlugZap,
  Search,
  Settings,
  SquarePen,
  Wrench,
} from "lucide-react";
import type { Conversation, UserProfile } from "../types";
import { ChatHistoryList } from "./ChatHistoryList";
import packageJson from "../../package.json";

const MIN_SIDEBAR_WIDTH = 260;
const MAX_SIDEBAR_WIDTH = 440;
const MOBILE_MEDIA_QUERY = "(max-width: 860px)";

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
    props.userProfile?.preferredName || props.userProfile?.fullName || "OpenMindAI User";
  const [profileMenuOpen, setProfileMenuOpen] = useState(false);
  const [resizing, setResizing] = useState(false);
  const [mobileLayout, setMobileLayout] = useState(() => window.matchMedia(MOBILE_MEDIA_QUERY).matches);
  const [mobileOpen, setMobileOpen] = useState(false);
  const profileMenuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const media = window.matchMedia(MOBILE_MEDIA_QUERY);
    const sync = () => {
      setMobileLayout(media.matches);
      if (!media.matches) setMobileOpen(false);
    };
    sync();
    media.addEventListener("change", sync);
    return () => media.removeEventListener("change", sync);
  }, []);

  useEffect(() => {
    const openConversation = (event: Event) => {
      const id = (event as CustomEvent<string>).detail;
      if (id) props.onOpenConversation(id);
    };
    const openModels = () => props.onOpenModels();
    const openSearch = () => props.onOpenSearch();
    window.addEventListener("openmindai:open-conversation", openConversation);
    window.addEventListener("openmindai:open-models", openModels);
    window.addEventListener("openmindai:open-search", openSearch);
    return () => {
      window.removeEventListener("openmindai:open-conversation", openConversation);
      window.removeEventListener("openmindai:open-models", openModels);
      window.removeEventListener("openmindai:open-search", openSearch);
    };
  }, [props]);

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

  const runMobileAction = (action: () => void) => {
    action();
    if (mobileLayout) setMobileOpen(false);
  };

  const baseSidebarClass = props.collapsed
    ? "sidebar collapsed"
    : resizing
      ? "sidebar resizing"
      : "sidebar";
  const sidebarClass = mobileOpen ? `${baseSidebarClass} mobile-open` : baseSidebarClass;
  const mobileChatOpen = mobileLayout && props.view === "chat" && props.activeId !== null;

  return (
    <>
      <button
        type="button"
        className={mobileChatOpen ? "mobile-menu-button mobile-back-button" : "mobile-menu-button"}
        aria-label={mobileChatOpen ? "Back to conversations" : mobileOpen ? "Close navigation" : "Open navigation"}
        aria-expanded={mobileChatOpen ? undefined : mobileOpen}
        onClick={() => {
          if (mobileChatOpen) {
            props.onNewChat();
            return;
          }
          setMobileOpen((value) => !value);
        }}
      >
        {mobileChatOpen ? <ArrowLeft size={20} /> : <Menu size={20} />}
      </button>

      {mobileChatOpen ? (
        <button
          type="button"
          className="mobile-chat-menu-button"
          aria-label="Open navigation"
          onClick={() => setMobileOpen(true)}
        >
          <MoreHorizontal size={20} />
        </button>
      ) : null}

      {mobileOpen ? (
        <button
          type="button"
          className="mobile-sidebar-backdrop"
          aria-label="Close navigation"
          onClick={() => setMobileOpen(false)}
        />
      ) : null}

      <aside
        className={sidebarClass}
        style={props.collapsed ? undefined : { width: props.width }}
        aria-hidden={mobileLayout && !mobileOpen ? true : undefined}
      >
        <div className="sidebar-header">
          <div className="brand">
            <img className="brand-mark" src="/icon.png" alt="OpenMindAI" />
            {!props.collapsed ? (
              <div>
                <strong>OpenMindAI</strong>
                <small>Your local AI workspace</small>
              </div>
            ) : null}
          </div>
          {!props.collapsed ? (
            <div className="sidebar-header-actions">
              <button
                className="icon-button"
                title="Search chats (Ctrl+K)"
                onClick={() => runMobileAction(props.onOpenSearch)}
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

        <button
          className="primary-command"
          onClick={() => runMobileAction(props.onNewChat)}
          title="New chat"
        >
          <SquarePen size={18} /> {!props.collapsed ? "New Chat" : null}
        </button>

        <nav className="sidebar-nav mobile-primary-nav">
          <button
            className={props.view === "projects" ? "nav-button active" : "nav-button"}
            onClick={() => runMobileAction(props.onOpenProjects)}
            title="Projects"
          >
            <FolderKanban size={18} /> {!props.collapsed ? "Projects" : null}
          </button>
          <button className="nav-button" onClick={() => runMobileAction(props.onOpenModels)} title="Models">
            <Bot size={18} /> {!props.collapsed ? "Models" : null}
          </button>
          <button
            className="nav-button mobile-connected-apps-nav"
            onClick={() => runMobileAction(() => props.onOpenSettings("apps"))}
            title="Connected Apps"
          >
            <PlugZap size={18} /> {!props.collapsed ? "Connected Apps" : null}
          </button>
          <button
            className="nav-button"
            onClick={() => runMobileAction(props.onOpenLibrary)}
            title="Files & Artifacts"
          >
            <FileBox size={18} /> {!props.collapsed ? "Artifacts" : null}
          </button>
          <button
            className={props.view === "settings" ? "nav-button active" : "nav-button"}
            onClick={() => runMobileAction(() => props.onOpenSettings())}
            title="Settings"
          >
            <Settings size={18} /> {!props.collapsed ? "Settings" : null}
          </button>
          <button
            className="nav-button mobile-help-nav"
            onClick={() => runMobileAction(() => props.onOpenSettings("about"))}
            title="Help & Docs"
          >
            <CircleHelp size={18} /> {!props.collapsed ? "Help & Docs" : null}
          </button>
          <button
            className="nav-button mobile-about-nav"
            onClick={() => runMobileAction(() => props.onOpenSettings("about"))}
            title="About OpenMindAI"
          >
            <Info size={18} /> {!props.collapsed ? "About OpenMindAI" : null}
          </button>

          <button
            className="nav-button desktop-secondary-nav"
            onClick={() => runMobileAction(props.onOpenLibrary)}
            title="Library"
          >
            <LibraryBig size={18} /> {!props.collapsed ? "Library" : null}
          </button>
          <button
            className="nav-button desktop-secondary-nav"
            onClick={() => runMobileAction(props.onOpenTools)}
            title="Tools"
          >
            <Wrench size={18} /> {!props.collapsed ? "Tools" : null}
          </button>
          <button
            className="nav-button desktop-secondary-nav"
            onClick={() => runMobileAction(props.onOpenLibrary)}
            title="Files & Artifacts"
          >
            <FolderOpen size={18} /> {!props.collapsed ? "Files & Artifacts" : null}
          </button>
          <button
            className="nav-button desktop-secondary-nav"
            onClick={() => runMobileAction(() => props.onOpenSettings())}
            title="More"
          >
            <MoreHorizontal size={18} /> {!props.collapsed ? "More" : null}
          </button>
        </nav>

        <ChatHistoryList
          conversations={props.conversations}
          activeId={props.activeId}
          collapsed={props.collapsed}
          onOpen={(id) => runMobileAction(() => props.onOpenConversation(id))}
          onRename={(conversation) => runMobileAction(() => props.onRename(conversation))}
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
                props.collapsed
                  ? runMobileAction(() => props.onOpenSettings())
                  : setProfileMenuOpen((value) => !value)
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
                  <span className="profile-role">Local Profile · v{packageJson.version}</span>
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
                  runMobileAction(() => props.onOpenSettings("general"));
                  setProfileMenuOpen(false);
                }}
              >
                Settings
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  runMobileAction(() => props.onOpenSettings("personalization"));
                  setProfileMenuOpen(false);
                }}
              >
                Personalization
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  runMobileAction(() => props.onOpenSettings("appearance"));
                  setProfileMenuOpen(false);
                }}
              >
                Appearance
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  runMobileAction(() => props.onOpenSettings("about"));
                  setProfileMenuOpen(false);
                }}
              >
                About
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

      <nav className="mobile-tabbar" aria-label="Mobile navigation">
        <button
          type="button"
          className={props.view === "chat" ? "mobile-tab active" : "mobile-tab"}
          onClick={props.onNewChat}
        >
          <MessageSquareText size={20} />
          <span>Chats</span>
        </button>
        <button
          type="button"
          className={props.view === "projects" ? "mobile-tab active" : "mobile-tab"}
          onClick={props.onOpenProjects}
        >
          <FolderKanban size={20} />
          <span>Projects</span>
        </button>
        <button type="button" className="mobile-tab" onClick={props.onOpenModels}>
          <Database size={20} />
          <span>Models</span>
        </button>
        <button
          type="button"
          className={props.view === "settings" ? "mobile-tab active" : "mobile-tab"}
          onClick={() => props.onOpenSettings()}
        >
          <Settings size={20} />
          <span>Settings</span>
        </button>
      </nav>
    </>
  );
}
