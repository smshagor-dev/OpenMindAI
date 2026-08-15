import type { Conversation } from "../types";

export interface ConversationGroup {
  label: string;
  conversations: Conversation[];
}

export function groupConversationsByDate(conversations: Conversation[]): ConversationGroup[] {
  const pinned = conversations.filter((conversation) => conversation.pinned);
  const rest = conversations.filter((conversation) => !conversation.pinned);

  const startOfDay = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const today = startOfDay(new Date());
  const oneDay = 24 * 60 * 60 * 1000;

  const buckets: ConversationGroup[] = [
    { label: "Today", conversations: [] },
    { label: "Yesterday", conversations: [] },
    { label: "Previous 7 Days", conversations: [] },
    { label: "Previous 30 Days", conversations: [] },
    { label: "Older", conversations: [] },
  ];

  for (const conversation of rest) {
    const updated = startOfDay(new Date(conversation.updatedAt));
    const daysAgo = Math.round((today - updated) / oneDay);
    if (daysAgo <= 0) buckets[0].conversations.push(conversation);
    else if (daysAgo === 1) buckets[1].conversations.push(conversation);
    else if (daysAgo <= 7) buckets[2].conversations.push(conversation);
    else if (daysAgo <= 30) buckets[3].conversations.push(conversation);
    else buckets[4].conversations.push(conversation);
  }

  const groups = buckets.filter((group) => group.conversations.length > 0);
  if (pinned.length > 0) groups.unshift({ label: "Pinned", conversations: pinned });
  return groups;
}
