// Release files have no version in their names (see the release job in
// .github/workflows/build.yml), so these links always serve the newest release.
export const RELEASES_URL = "https://github.com/DereckAn/Quakboard/releases";
export const LATEST_DOWNLOAD = `${RELEASES_URL}/latest/download`;

export const download = (file: string) => `${LATEST_DOWNLOAD}/${file}`;
export const ISSUES_URL = "https://github.com/DereckAn/Quakboard/issues";
