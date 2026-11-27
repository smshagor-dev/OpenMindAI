export const CLOUD_CONNECT_PROMPT_EVENT = "openmindai:cloud-connect-prompt";

export type CloudConnectPromptDetail = { reason: string };

/** Only credential and transport failures interrupt the user with the key dialog. */
export function cloudConnectionPromptReason(error: unknown): string | null {
  const message = typeof error === "string" ? error : error instanceof Error ? error.message : String(error ?? "");
  if (/\b(?:401|403|unauthori[sz]ed|forbidden|invalid api key|credential(?:s)? (?:is|are) (?:invalid|missing)|api key (?:is )?(?:invalid|missing|required)|connect an? (?:openmindai )?(?:cloud )?(?:account|api key)|not connected)\b/i.test(message))
    return "OpenMindAI Cloud needs a valid API key. Add or replace your key to continue.";
  if (/\b(?:cloud api is unreachable|failed to connect|connection refused|network is unreachable|name or service not known|dns|timed? out|request failed)\b/i.test(message))
    return "OpenMindAI Cloud could not be reached. Check your internet connection and connect with your API key.";
  return null;
}

export function promptForCloudConnection(reason: string): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new window.CustomEvent<CloudConnectPromptDetail>(CLOUD_CONNECT_PROMPT_EVENT, { detail: { reason } }));
}
