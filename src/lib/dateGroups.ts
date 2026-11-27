import type { Conversation } from "../types";

export interface ConversationGroup {
  label: string;
  conversations: Conversation[];
}

export function groupConversationsByDate(conversations: Conversation[]): ConversationGroup[] {
  const pinned = conversations.filter((conversation) => conversation.pinned);
  const rest = conversations.filter((conversation) => !conversation.pinned);

  // Compare local calendar dates, not elapsed milliseconds (DST days vary in length).
  const calendarDay = (date: Date) =>
    Number.isNaN(date.getTime()) ? null : Math.floor(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / 86_400_000);
  const today = calendarDay(new Date())!;

  const buckets: ConversationGroup[] = [
    { label: "Today", conversations: [] },
    { label: "Yesterday", conversations: [] },
    { label: "Previous 7 Days", conversations: [] },
    { label: "Previous 30 Days", conversations: [] },
    { label: "Older", conversations: [] },
  ];

  for (const conversation of rest) {
    const updated = calendarDay(new Date(conversation.updatedAt));
    const daysAgo = updated === null ? Number.POSITIVE_INFINITY : today - updated;
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
