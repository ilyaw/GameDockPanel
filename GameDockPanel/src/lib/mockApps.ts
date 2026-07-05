import type { DockApp } from "./types";

/**
 * Mock dock contents. `iconUrl` points at Dashboard Icons via
 * raw.githubusercontent.com (see tauri-glass-dock skill — cdn.jsdelivr.net
 * currently 404s on this repo because it exceeded jsDelivr's package size
 * limit). Real process-monitoring/native-icon extraction replaces this in
 * a later pass.
 */
export const initialApps: DockApp[] = [
  {
    id: "discord",
    name: "Discord",
    iconUrl:
      "https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/png/discord.png",
    isActive: true,
    color: "text-indigo-400",
  },
  {
    id: "steam",
    name: "Steam",
    iconUrl:
      "https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/png/steam.png",
    isActive: true,
    color: "text-sky-400",
  },
  {
    id: "spotify",
    name: "Spotify",
    iconUrl:
      "https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/png/spotify.png",
    isActive: true,
    color: "text-green-400",
  },
  {
    id: "minecraft",
    name: "Minecraft",
    iconUrl:
      "https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/png/minecraft.png",
    isActive: false,
    color: "text-lime-400",
  },
  {
    id: "obs-studio",
    name: "OBS Studio",
    iconUrl:
      "https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/png/obs-studio.png",
    isActive: false,
    color: "text-red-400",
  },
  {
    id: "epic-games",
    name: "Epic Games",
    iconUrl:
      "https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/png/epic-games.png",
    isActive: false,
    color: "text-fuchsia-400",
  },
];
