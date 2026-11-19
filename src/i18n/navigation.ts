export const navigationMessages = {
  English: {
    tagline: "Your AI. Your Models. Your Way.", search: "Search chats (Ctrl+K)",
    collapse: "Collapse sidebar", expand: "Expand sidebar", newChat: "New Chat",
    library: "Library", models: "Models", datasets: "Datasets", tools: "Tools",
    research: "Research Studio", studio3d: "3D Studio", web: "Web Search",
    files: "Files & Artifacts", projects: "Projects", more: "More",
    profile: "Profile options", settings: "Settings", personalization: "Personalization",
    appearance: "Appearance", about: "About",
  },
  Bangla: {
    tagline: "আপনার এআই। আপনার মডেল। আপনার পছন্দ।", search: "চ্যাট খুঁজুন (Ctrl+K)",
    collapse: "সাইডবার ছোট করুন", expand: "সাইডবার খুলুন", newChat: "নতুন চ্যাট",
    library: "লাইব্রেরি", models: "মডেল", datasets: "ডেটাসেট", tools: "টুলস",
    research: "রিসার্চ স্টুডিও", studio3d: "৩ডি স্টুডিও", web: "ওয়েব অনুসন্ধান",
    files: "ফাইল ও আর্টিফ্যাক্ট", projects: "প্রজেক্ট", more: "আরও",
    profile: "প্রোফাইল বিকল্প", settings: "সেটিংস", personalization: "ব্যক্তিগতকরণ",
    appearance: "চেহারা", about: "সম্পর্কে",
  },
  Hindi: {
    tagline: "आपका AI। आपके मॉडल। आपकी पसंद।", search: "चैट खोजें (Ctrl+K)",
    collapse: "साइडबार समेटें", expand: "साइडबार खोलें", newChat: "नई चैट",
    library: "लाइब्रेरी", models: "मॉडल", datasets: "डेटासेट", tools: "टूल्स",
    research: "रिसर्च स्टूडियो", studio3d: "3D स्टूडियो", web: "वेब खोज",
    files: "फ़ाइलें और आर्टिफ़ैक्ट", projects: "प्रोजेक्ट", more: "अधिक",
    profile: "प्रोफ़ाइल विकल्प", settings: "सेटिंग्स", personalization: "निजीकरण",
    appearance: "दिखावट", about: "परिचय",
  },
} as const;

export type NavigationKey = keyof typeof navigationMessages.English;
export function navigationText(language: string | undefined, key: NavigationKey): string {
  const locale = language === "Bangla" || language === "Hindi" ? language : "English";
  return navigationMessages[locale][key] ?? navigationMessages.English[key];
}
