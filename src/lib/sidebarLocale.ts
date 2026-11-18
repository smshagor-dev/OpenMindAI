// Initial, explicitly scoped navigation translations. Other application surfaces
// retain English until their own locale catalogs are implemented.
export const sidebarMessages = {
  English: {
    search: "Search chats (Ctrl+K)",
    collapse: "Collapse sidebar",
    expand: "Expand sidebar",
    newChat: "New Chat",
    library: "Library",
    models: "Models",
    datasets: "Datasets",
    tools: "Tools",
    research: "Research Studio",
    studio3d: "3D Studio",
    webSearch: "Web Search",
    files: "Files & Artifacts",
    projects: "Projects",
    more: "More",
    profileOptions: "Profile options",
    settings: "Settings",
    personalization: "Personalization",
    appearance: "Appearance",
    about: "About",
  },
  Bangla: {
    search: "চ্যাট খুঁজুন (Ctrl+K)",
    collapse: "সাইডবার সংকুচিত করুন",
    expand: "সাইডবার প্রসারিত করুন",
    newChat: "নতুন চ্যাট",
    library: "লাইব্রেরি",
    models: "মডেল",
    datasets: "ডেটাসেট",
    tools: "টুলস",
    research: "গবেষণা স্টুডিও",
    studio3d: "৩ডি স্টুডিও",
    webSearch: "ওয়েব অনুসন্ধান",
    files: "ফাইল ও আর্টিফ্যাক্ট",
    projects: "প্রজেক্ট",
    more: "আরও",
    profileOptions: "প্রোফাইলের বিকল্প",
    settings: "সেটিংস",
    personalization: "ব্যক্তিগতকরণ",
    appearance: "চেহারা",
    about: "সম্পর্কে",
  },
  Hindi: {
    search: "चैट खोजें (Ctrl+K)",
    collapse: "साइडबार छोटा करें",
    expand: "साइडबार खोलें",
    newChat: "नई चैट",
    library: "लाइब्रेरी",
    models: "मॉडल",
    datasets: "डेटासेट",
    tools: "टूल्स",
    research: "रिसर्च स्टूडियो",
    studio3d: "3D स्टूडियो",
    webSearch: "वेब खोज",
    files: "फ़ाइलें और आर्टिफ़ैक्ट",
    projects: "प्रोजेक्ट",
    more: "और",
    profileOptions: "प्रोफ़ाइल विकल्प",
    settings: "सेटिंग्स",
    personalization: "वैयक्तिकरण",
    appearance: "दिखावट",
    about: "परिचय",
  },
} as const;

export type SidebarTextKey = keyof typeof sidebarMessages.English;

export function sidebarText(language: string | null | undefined, key: SidebarTextKey): string {
  const catalog = language === "Bangla" || language === "Hindi"
    ? sidebarMessages[language]
    : sidebarMessages.English;
  return catalog[key] || sidebarMessages.English[key];
}
