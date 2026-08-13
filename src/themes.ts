import graphite from "./torrent-theme-2.css?url";
import blackout from "./torrent-theme-3.css?url";
import paperlike from "./torrent-theme-4.css?url";

export interface ThemeDef {
  id: string;
  name: string;
  url: string | null;
}


export const THEMES: ThemeDef[] = [
  { id: "default", name: "Quantum", url: null },
  { id: "blackout", name: "Blackout", url: blackout },
  { id: "graphene", name: "Graphene", url: graphite },
  { id: "paperlike", name: "Paperlike", url: paperlike },
];

const LINK_ID = "theme-override";

export function applyTheme(id: string): void {
  const theme = THEMES.find(t => t.id === id) ?? THEMES[0];
  const existing = document.getElementById(LINK_ID) as HTMLLinkElement | null;

  if (!theme.url) {
    existing?.remove();
    return;
  }
  if (existing) {
    if (!existing.href.endsWith(theme.url)) existing.href = theme.url;
    return;
  }


  const link = document.createElement("link");
  link.id = LINK_ID;
  link.rel = "stylesheet";
  link.href = theme.url;
  document.head.appendChild(link);
}
