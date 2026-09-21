export interface LanguageInfo {
  name: string;
  svgPath: string;
}

// NOTE: let Vite bundle the icons and hand back their URLs. A hand-built
// `asset://localhost/...` URL only resolves on some platforms (Windows needs
// `http://asset.localhost/`) and points at the filesystem, not the bundle.
const svgUrls = import.meta.glob<string>("/src/assets/svg/*.svg", {
  eager: true,
  query: "?url",
  import: "default",
});

export function getLanguageInfo(language: string | null): LanguageInfo | null {
  if (!language) return null;

  const assetPath = (name: string) => svgUrls[`/src/assets/svg/${name}.svg`];

  const languages: Record<string, LanguageInfo> = {
    javascript: {
      name: "JavaScript",
      svgPath: assetPath("js"),
    },
    typescript: {
      name: "TypeScript",
      svgPath: assetPath("ts"),
    },
    python: {
      name: "Python",
      svgPath: assetPath("python"),
    },
    rust: {
      name: "Rust",
      svgPath: assetPath("rust"),
    },
    go: {
      name: "Go",
      svgPath: assetPath("go"),
    },
    java: {
      name: "Java",
      svgPath: assetPath("java"),
    },
    cpp: {
      name: "C++",
      svgPath: assetPath("cpp"),
    },
    html: {
      name: "HTML",
      svgPath: assetPath("html"),
    },
    css: {
      name: "CSS",
      svgPath: assetPath("css"),
    },
    svelte: {
      name: "Svelte",
      svgPath: assetPath("svelte"),
    },
    vue: {
      name: "Vue",
      svgPath: assetPath("vue"),
    },
    react: {
      name: "React",
      svgPath: assetPath("react"),
    },
    json: {
      name: "JSON",
      svgPath: assetPath("json"),
    },
    markdown: {
      name: "Markdown",
      svgPath: assetPath("md"),
    },
    sql: {
      name: "SQL",
      svgPath: assetPath("sql"),
    },
    php: {
      name: "PHP",
      svgPath: assetPath("php"),
    },
    ruby: {
      name: "Ruby",
      svgPath: assetPath("ruby"),
    },
    bash: {
      name: "Bash",
      svgPath: assetPath("bash"),
    },
    swift: {
      name: "Swift",
      svgPath: assetPath("swift"),
    },
    dart: {
      name: "Dart",
      svgPath: assetPath("dart"),
    },
    docker: {
      name: "Docker",
      svgPath: assetPath("docker"),
    },
    kotlin: {
      name: "Kotlin",
      svgPath: assetPath("kotlin"),
    },
    csharp: {
      name: "C#",
      svgPath: assetPath("csharp"),
    },
  };

  // No icon for unknown languages; callers fall back to the generic code icon.
  return languages[language.toLowerCase()] ?? null;
}
