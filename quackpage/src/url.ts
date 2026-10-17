// The site is served from https://dereckan.github.io/Quakboard/, so every
// internal link and asset needs the base path in front.
export const url = (path: string) =>
  import.meta.env.BASE_URL.replace(/\/$/, "") + path;
