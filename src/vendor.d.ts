declare module "lucide-react" {
  import type { ComponentType, SVGProps } from "react";

  export type LucideProps = SVGProps<SVGSVGElement> & {
    size?: string | number;
    strokeWidth?: string | number;
    absoluteStrokeWidth?: boolean;
  };
  export type LucideIcon = ComponentType<LucideProps>;

  export const Activity: LucideIcon;
  export const Archive: LucideIcon;
  export const ArrowLeft: LucideIcon;
  export const ArrowUp: LucideIcon;
  export const BookOpenCheck: LucideIcon;
  export const Bot: LucideIcon;
  export const Boxes: LucideIcon;
  export const Brain: LucideIcon;
  export const BriefcaseBusiness: LucideIcon;
  export const Check: LucideIcon;
  export const CheckCheck: LucideIcon;
  export const CheckCircle2: LucideIcon;
  export const ChevronDown: LucideIcon;
  export const ChevronRight: LucideIcon;
  export const CircleAlert: LucideIcon;
  export const Clock3: LucideIcon;
  export const Cloud: LucideIcon;
  export const Code2: LucideIcon;
  export const Copy: LucideIcon;
  export const Cpu: LucideIcon;
  export const Database: LucideIcon;
  export const DatabaseZap: LucideIcon;
  export const Download: LucideIcon;
  export const ExternalLink: LucideIcon;
  export const Eye: LucideIcon;
  export const FileCode: LucideIcon;
  export const FileCode2: LucideIcon;
  export const FilePlus2: LucideIcon;
  export const Files: LucideIcon;
  export const FileSearch: LucideIcon;
  export const FileText: LucideIcon;
  export const FileType: LucideIcon;
  export const Folder: LucideIcon;
  export const FolderKanban: LucideIcon;
  export const FolderOpen: LucideIcon;
  export const FolderPlus: LucideIcon;
  export const Gauge: LucideIcon;
  export const Github: LucideIcon;
  export const Globe: LucideIcon;
  export const HardDrive: LucideIcon;
  export const Hash: LucideIcon;
  export const History: LucideIcon;
  export const Image: LucideIcon;
  export const Info: LucideIcon;
  export const Keyboard: LucideIcon;
  export const LibraryBig: LucideIcon;
  export const Link2: LucideIcon;
  export const Loader2: LucideIcon;
  export const LoaderCircle: LucideIcon;
  export const LockKeyhole: LucideIcon;
  export const Mail: LucideIcon;
  export const MessageSquare: LucideIcon;
  export const MessageSquarePlus: LucideIcon;
  export const MessagesSquare: LucideIcon;
  export const Mic: LucideIcon;
  export const MicOff: LucideIcon;
  export const Minus: LucideIcon;
  export const MoreHorizontal: LucideIcon;
  export const Music2: LucideIcon;
  export const Palette: LucideIcon;
  export const PanelLeftClose: LucideIcon;
  export const PanelLeftOpen: LucideIcon;
  export const Paperclip: LucideIcon;
  export const PauseCircle: LucideIcon;
  export const Pencil: LucideIcon;
  export const PencilLine: LucideIcon;
  export const Pin: LucideIcon;
  export const PinOff: LucideIcon;
  export const Play: LucideIcon;
  export const Plug: LucideIcon;
  export const PlugZap: LucideIcon;
  export const Plus: LucideIcon;
  export const RefreshCw: LucideIcon;
  export const RotateCcw: LucideIcon;
  export const Save: LucideIcon;
  export const Search: LucideIcon;
  export const Send: LucideIcon;
  export const Server: LucideIcon;
  export const Settings: LucideIcon;
  export const Shield: LucideIcon;
  export const ShieldCheck: LucideIcon;
  export const SlidersHorizontal: LucideIcon;
  export const Sparkles: LucideIcon;
  export const Square: LucideIcon;
  export const SquarePen: LucideIcon;
  export const StopCircle: LucideIcon;
  export const TableProperties: LucideIcon;
  export const TerminalSquare: LucideIcon;
  export const Trash2: LucideIcon;
  export const TriangleAlert: LucideIcon;
  export const Unplug: LucideIcon;
  export const UploadCloud: LucideIcon;
  export const User: LucideIcon;
  export const Video: LucideIcon;
  export const Volume2: LucideIcon;
  export const WandSparkles: LucideIcon;
  export const Wrench: LucideIcon;
  export const X: LucideIcon;
  export const XCircle: LucideIcon;
}

declare module "highlight.js/lib/core" {
  const hljs: {
    registerLanguage: (name: string, language: unknown) => void;
    getLanguage: (name: string) => unknown;
    highlight: (code: string, options: { language: string; ignoreIllegals?: boolean }) => {
      value: string;
    };
    highlightAuto: (code: string) => { value: string };
  };
  export default hljs;
}

declare module "highlight.js/lib/languages/python" {
  const language: unknown;
  export default language;
}

declare module "highlight.js/lib/languages/typescript" {
  const language: unknown;
  export default language;
}

declare module "highlight.js/lib/languages/javascript" {
  const language: unknown;
  export default language;
}

declare module "highlight.js/lib/languages/rust" {
  const language: unknown;
  export default language;
}

declare module "highlight.js/lib/languages/json" {
  const language: unknown;
  export default language;
}

declare module "highlight.js/lib/languages/bash" {
  const language: unknown;
  export default language;
}

declare module "highlight.js/lib/languages/xml" {
  const language: unknown;
  export default language;
}

declare module "highlight.js/lib/languages/css" {
  const language: unknown;
  export default language;
}
