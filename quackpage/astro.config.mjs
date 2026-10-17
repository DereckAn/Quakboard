// @ts-check
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";

// https://astro.build/config
export default defineConfig({
  site: "https://dereckan.github.io",
  base: "/Quakboard",
  // the HTML compressor drops the space between a line of text and a link on
  // the next line ("on<a>GitHub"), so keep the HTML as written
  compressHTML: false,
  vite: {
    plugins: [tailwindcss()],
  },
});
