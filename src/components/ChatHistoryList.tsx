import { useEffect, useRef, useState } from "react";
import { MoreHorizontal, Pencil, Pin, PinOff, Archive, Trash2, Copy } from "lucide-react";
import type { Conversation } from "../types";
import { groupConversationsByDate } from "../lib/dateGroups";

export function ChatHistoryList(props: {
  conversations: Conversation[];
  activeId: string | null;
  collapsed: boolean;
  onOpen: (id: string) => void;
  onRename: (conversation: Conversation) => void;
  onTogglePin: (conversation: Conversation) => void;
  onArchive: (id: string) => void;
  onDelete: (id: string) => void;
  onDuplicate: (conversation: Conversation) => void;
}) {
  const groups = groupConversationsByDate(props.conversations);
  const [menuFor, setMenuFor] = useState<string | null>(null);

  if (props.collapsed) {
    return (
      <nav className="conversation-list collapsed" aria-label="Chat history">
        {props.conversations.slice(0, 20).map((conversation) => (
          <button
            key={conversation.id}
            className={conversation.id === props.activeId ? "conversation-icon active" : "conversation-icon"}
            title={conversation.title}
            onClick={() => props.onOpen(conversation.id)}
          >
            {conversation.title.charAt(0).toUpperCase() || "?"}
          </button>
        ))}
      </nav>
    );
  }

  return (
    <nav className="conversation-list" aria-label="Chat history">
      {groups.map((group) => (
        <div className="conversation-group" key={group.label}>
          <h3>{group.label}</h3>
          {group.conversations.map((conversation) => (
            <ChatHistoryItem
              key={conversation.id}
              conversation={conversation}
              active={conversation.id === props.activeId}
              menuOpen={menuFor === conversation.id}
              onOpen={() => props.onOpen(conversation.id)}
              onOpenMenu={() => setMenuFor(conversation.id)}
              onCloseMenu={() => setMenuFor((current) => (current === conversation.id ? null : current))}
              onRename={() => props.onRename(conversation)}
              onTogglePin={() => props.onTogglePin(conversation)}
              onArchive={() => props.onArchive(conversation.id)}
              onDelete={() => props.onDelete(conversation.id)}
              onDuplicate={() => props.onDuplicate(conversation)}
            />
          ))}
        </div>
      ))}
      {props.conversations.length === 0 ? <p className="muted conversation-empty">No chats yet.</p> : null}
    </nav>
  );
}

function ChatHistoryItem(props: {
  conversation: Conversation;
  active: boolean;
  menuOpen: boolean;
  onOpen: () => void;
  onOpenMenu: () => void;
  onCloseMenu: () => void;
  onRename: () => void;
  onTogglePin: () => void;
  onArchive: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
}) {
  const menuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!props.menuOpen) return;
    const onClickOutside = (event: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        props.onCloseMenu();
      }
    };
    window.addEventListener("mousedown", onClickOutside);
    return () => window.removeEventListener("mousedown", onClickOutside);
  }, [props]);

  return (
    <div className={props.active ? "conversation active" : "conversation"} ref={menuRef}>
      <button className="conversation-open" onClick={props.onOpen}>
        <span>{props.conversation.title}</span>
        {props.conversation.pinned ? <Pin size={13} /> : null}
      </button>
      <button
        className="conversation-menu-trigger"
        title="Chat options"
        onClick={(event) => {
          event.stopPropagation();
          if (props.menuOpen) {
            props.onCloseMenu();
          } else {
            props.onOpenMenu();
          }
        }}
      >
        <MoreHorizontal size={16} />
      </button>
      {props.menuOpen ? (
        <div className="conversation-menu" role="menu">
          <button role="menuitem" onClick={props.onRename}>
            <Pencil size={14} /> Rename
          </button>
          <button role="menuitem" onClick={props.onTogglePin}>
            {props.conversation.pinned ? <PinOff size={14} /> : <Pin size={14} />}
            {props.conversation.pinned ? "Unpin" : "Pin"}
          </button>
          <button role="menuitem" onClick={props.onDuplicate}>
            <Copy size={14} /> Duplicate
          </button>
          <button role="menuitem" onClick={props.onArchive}>
            <Archive size={14} /> Archive
          </button>
          <button role="menuitem" className="danger" onClick={props.onDelete}>
            <Trash2 size={14} /> Delete
          </button>
        </div>
      ) : null}
    </div>
  );
}
