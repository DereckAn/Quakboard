# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Global Clipboard Manager is a production-ready clipboard management application built with Tauri v2 (Rust backend + Svelte 5 frontend). It allows users to manage clipboard history with support for text, code, links, colors, images, and files.

**Current Status**: v1.1 - Fully functional with advanced features implemented including:
- ✅ Clipboard monitoring and auto-save
- ✅ Smart content detection (text, code, colors, links, files)
- ✅ Efficient pagination with infinite scroll
- ✅ Dynamic global hotkeys
- ✅ Full-text search with FTS5 (SQLite)
- ✅ Search highlighting in results
- ✅ Favorites and snippets
- ✅ Date-based grouping
- ✅ Advanced settings (item limits, retention policies, memory management)
- ✅ Cross-platform window customization
- ✅ Privacy-first local storage

## Technology Stack

### Frontend
- **Framework**: Tauri v2 with SvelteKit + Svelte 5
- **Package Manager**: Bun
- **Language**: TypeScript
- **Styling**: TailwindCSS v4 with `@theme` syntax
- **Reactivity**: Svelte 5 Runes (`$state`, `$derived`, `$derived.by()`, `$effect`)
- **Adapter**: `@sveltejs/adapter-static` (SPA mode for Tauri)
- **Local Storage**: SQLite (via Tauri commands with pagination)

### Backend (Rust/Tauri)
- **Framework**: Tauri v2.9.1
- **Language**: Rust 1.70+
- **Database**: SQLite with `rusqlite`
- **Clipboard**: Background monitoring with 500ms polling
- **Plugins**:
  - `tauri-plugin-opener` - Open files and URLs
  - `tauri-plugin-notification` - Native notifications
  - `tauri-plugin-global-shortcut` - System-wide keyboard shortcuts

### Build Tools
- **Frontend Build**: Vite 6 with SvelteKit
- **Backend Build**: Cargo
- **Dev Server**: Port 1420 (fixed for Tauri)
- **HMR**: Port 1421

## Architecture

The application follows **Clean Architecture** with clear layer separation:

```
┌─────────────────────────────────────────────────────────┐
│                    PRESENTATION                          │
│        (routes/ + lib/components/)                       │
│  • Svelte 5 components with Runes                       │
│  • No business logic                                    │
└──────────────────────┬──────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│                  APPLICATION                             │
│              (lib/stores/)                               │
│  • State management with Svelte 5 runes                 │
│  • Business logic orchestration                         │
│  • Pagination state management                          │
└──────────────────────┬──────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│                 INFRASTRUCTURE                           │
│           (lib/tauri/ + Rust backend)                    │
│  • Tauri command wrappers (TypeScript)                  │
│  • Repository implementations (Rust)                     │
│  • Database access with pagination (SQLite)             │
└──────────────────────┬──────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│                    DOMAIN                                │
│        (lib/types/ + Rust models)                        │
│  • Type definitions & interfaces                        │
│  • Business rules & validation                          │
└─────────────────────────────────────────────────────────┘
```

### Key Directories

- **`src/routes/`**: SvelteKit pages using file-based routing
  - `+page.svelte` - Main clipboard view with search and filters
  - `+layout.svelte` - App layout
  - `settings/+page.svelte` - Settings page with hotkey configuration

- **`src/lib/components/`**: Reusable Svelte components
  - `sidebar/` - Clipboard items list with infinite scroll
    - `Sidebar.svelte` - Main sidebar with scroll detection
    - `SidebarGroup.svelte` - Date-based grouping (Today, Yesterday, etc.)
    - `SidebarItem.svelte` - Individual item card
  - `content/` - Content viewer panel
    - `ContentViewer.svelte` - Main content display
    - `ItemInfo.svelte` - Metadata display
  - `header/` - Search and filters
    - `Header.svelte` - Top bar
    - `SearchBar.svelte` - Search input
    - `FilterDropdown.svelte` - Content type filters
  - `settings/` - Settings components
    - `HotkeyRecorder.svelte` - Keyboard shortcut recorder
  - `ui/` - Base UI components (Button, Input, Modal, etc.)

- **`src/lib/stores/`**: Svelte 5 runes-based stores
  - `clipboard.svelte.ts` - Clipboard state with pagination
  - `settings.svelte.ts` - App settings and hotkey management

- **`src/lib/tauri/`**: Typed wrappers for Tauri commands
  - `commands.ts` - All Tauri command wrappers
  - `storage.ts` - Repository pattern implementation

- **`src/lib/types/`**: TypeScript type definitions

- **`src/lib/utils/`**: Utility functions (cn, formatters)

- **`src-tauri/src/`**: Rust backend code
  - `clipboard/` - Clipboard monitoring
    - `monitor.rs` - Background clipboard watcher (500ms poll)
    - `operations.rs` - Read/write operations
    - `types.rs` - Content type detection (20+ code languages)
  - `commands/` - Tauri commands (API)
    - `clipboard.rs` - CRUD operations with pagination
    - `settings.rs` - Settings & dynamic hotkey management + cleanup commands
    - `colors.rs` - Color conversion (HEX, RGB, HSL, OKLCH)
    - `links.rs` - URL metadata fetching
    - `hotkey.rs` - Window visibility toggle
  - `db/` - Database layer
    - `repository.rs` - Data access with server-side pagination + FTS5 search
    - `fts_migration.rs` - Full-text search setup with SQLite FTS5
    - `models.rs` - Data models
    - `schema.rs` - SQLite schema initialization
  - `cleanup/` - Database maintenance
    - `mod.rs` - Old items cleanup, excess items cleanup, DB optimization
  - `shortcuts.rs` - Global hotkey registration and management
  - `lib.rs` - App initialization with cross-platform window customization

## Database Schema

### clipboard_items
```sql
CREATE TABLE clipboard_items (
    id TEXT PRIMARY KEY,
    content_type TEXT NOT NULL,      -- 'text', 'code', 'color', 'link', 'file', 'image'
    content_text TEXT,
    content_metadata TEXT,            -- JSON: { language, domain, formats, etc. }
    source_app TEXT,
    code_language TEXT,               -- Go, Rust, TypeScript, Python, etc.
    file_url TEXT,
    file_name TEXT,
    file_size_bytes INTEGER,
    file_mime_type TEXT,
    is_favorite INTEGER DEFAULT 0,
    is_snippet INTEGER DEFAULT 0,
    snippet_name TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    synced INTEGER DEFAULT 0,
    server_id TEXT
);

CREATE INDEX idx_created_at ON clipboard_items(created_at DESC);
CREATE INDEX idx_content_type ON clipboard_items(content_type);
```

**Location**: `~/Library/Application Support/com.dereckan.quakboard/clipboard.db` (macOS); `~/.local/share/com.dereckan.quakboard/clipboard.db` (Linux)

## Development Commands

### Frontend Development
```bash
# Start frontend dev server only
bun run dev

# Type checking
bun run check

# Type checking with watch mode
bun run check:watch

# Build frontend for production
bun run build

# Preview production build
bun run preview
```

### Full Application Development
```bash
# Start Tauri app with frontend (recommended for development)
bun run tauri dev

# Build production app
bun run tauri build

# Clean and rebuild
pkill -9 global_clipboard
lsof -ti:1420 | xargs kill -9
bun run tauri dev
```

### Backend (Rust) Development
```bash
# From src-tauri directory
cargo build          # Build
cargo test           # Run tests
cargo clippy         # Linting
cargo fmt            # Format code
cargo check          # Quick compile check
```

## Important Implementation Notes

### Svelte 5 Runes (New Reactivity System)

**DO NOT use legacy Svelte patterns**. This project uses Svelte 5 with the new runes system:

- `$state()` - reactive state variables
- `$derived()` - simple computed values
- `$derived.by()` - complex computed values with explicit function
- `$effect()` - side effects
- `$bindable()` - bindable props

**CRITICAL: $derived vs $derived.by()**
```typescript
// ❌ WRONG - Creates a function
const filtered = $derived(() => {
  return items.filter(item => item.id > 5)
})
// Usage: filtered() - needs parentheses

// ✅ CORRECT - Creates a value
const filtered = $derived.by(() => {
  return items.filter(item => item.id > 5)
})
// Usage: filtered - no parentheses
```

**Example of correct Svelte 5 store pattern:**
```typescript
// lib/stores/clipboard.svelte.ts
class ClipboardStore {
  items = $state<ClipboardItem[]>([])
  isLoading = $state(false)
  totalItems = $state(0)
  currentPage = $state(0)
  pageSize = $state(100)
  hasMore = $state(true)
  isLoadingMore = $state(false)

  async loadItems() {
    this.isLoading = true
    this.items = await tauriGetItemsPaginated(this.pageSize, 0)
    this.totalItems = await tauriCountItems()
    this.hasMore = this.items.length < this.totalItems
    this.isLoading = false
  }

  async loadMore() {
    if (!this.hasMore || this.isLoadingMore) return
    this.isLoadingMore = true
    this.currentPage++
    const offset = this.currentPage * this.pageSize
    const moreItems = await tauriGetItemsPaginated(this.pageSize, offset)
    this.items = [...this.items, ...moreItems]
    this.hasMore = this.items.length < this.totalItems
    this.isLoadingMore = false
  }
}

export const clipboardStore = new ClipboardStore()
```

### Pagination Implementation

**Strategy**: Server-side pagination for performance
- **Page size**: 50-100 items per request
- **Infinite scroll**: Automatic loading when scrolling near bottom
- **Memory efficient**: Only loads visible items
- **Optimized for mobile**: Prevents loading entire database

**Key Files**:
- `src-tauri/src/db/repository.rs` - `get_items_paginated()`, `search_items_paginated()`
- `src/lib/stores/clipboard.svelte.ts` - Pagination state and `loadMore()`
- `src/lib/components/sidebar/Sidebar.svelte` - Scroll detection

**Old functions removed** (use paginated versions):
- ❌ `get_items()` → ✅ `get_items_paginated()`
- ❌ `search_items()` → ✅ `search_items_paginated()`

### Dynamic Global Hotkeys

**Implementation**: Custom shortcuts module with clean registration pattern

**Key Files**:
- `src-tauri/src/shortcuts.rs` - Helper functions for hotkey management
- `src-tauri/src/commands/settings.rs` - `update_global_hotkey()` command
- `src-tauri/capabilities/default.json` - Permissions configuration

**Functions**:
```rust
// Register shortcut upon app start
pub fn register_shortcut_upon_start(app: &AppHandle, shortcut_str: &str) -> Result<(), String>

// Register shortcut dynamically at runtime (for settings changes)
pub fn register_shortcut(app: &AppHandle, shortcut_str: &str) -> Result<(), String>

// Unregister all shortcuts (before changing)
pub fn unregister_all_shortcuts(app: &AppHandle) -> Result<(), String>
```

**Process for changing hotkey**:
1. Save new hotkey to settings
2. Unregister all existing shortcuts
3. Register new shortcut with handler
4. No restart required!

### macOS-Specific Features

**Hide from Dock**:
```rust
// src-tauri/src/lib.rs
#[cfg(target_os = "macos")]
app.set_activation_policy(tauri::ActivationPolicy::Accessory);
```

**Auto-hide on focus loss**:
```rust
// src-tauri/src/lib.rs
.on_window_event(|window, event| {
    match event {
        tauri::WindowEvent::Focused(focused) => {
            if !focused {
                let _ = window.hide();
            }
        }
        _ => {}
    }
})
```

### TailwindCSS v4

Uses the new `@theme` syntax in CSS (not JavaScript config):

```css
/* src/app.css */
@import "tailwindcss";

@theme {
  --color-background: oklch(0.145 0 0);
  --color-surface: oklch(0.18 0 0);
  --color-border: oklch(0.25 0 0);
  /* ... */
}
```

### Tauri Integration

- Frontend communicates with Rust backend via `invoke()` from `@tauri-apps/api/core`
- All Tauri commands wrapped in typed functions in `lib/tauri/commands.ts`
- Use Repository pattern to abstract Tauri backend calls

**Example:**
```typescript
// lib/tauri/commands.ts
import { invoke } from '@tauri-apps/api/core'

export async function tauriGetItemsPaginated(
  limit: number,
  offset: number
): Promise<ClipboardItem[]> {
  const items = await invoke<any[]>("get_clipboard_items_paginated", {
    limit,
    offset,
  });
  return items.map(deserializeClipboardItem);
}
```

### SvelteKit Configuration

- Uses `adapter-static` with SPA mode (`ssr = false`)
- Fallback to `index.html` for client-side routing
- Path alias: `$lib` maps to `src/lib`

## Key Patterns and Conventions

### Component Props (Svelte 5)

Use the new `$props()` rune:

```svelte
<script lang="ts">
  interface Props {
    title: string
    count?: number
  }

  let { title, count = 0 }: Props = $props()
</script>
```

### Two-Way Binding

Use `$bindable()` for two-way bound props:

```svelte
<script lang="ts">
  let { value = $bindable('') }: { value: string } = $props()
</script>

<input bind:value />
```

### Snippets (Not Slots)

Svelte 5 uses snippets instead of slots:

```svelte
{@render children?.()}
```

### Class Merging

Use the `cn()` utility for combining Tailwind classes:

```svelte
<div class={cn('base-classes', conditionalClasses, className)}>
```

### Date-Based Grouping

Items are grouped by date in the sidebar:

```typescript
// lib/components/sidebar/SidebarGroup.svelte
const groups = ["Today", "Yesterday", "This Week", "This Month", "Older"]
```

## Development Workflow

### Working with Frontend Changes

1. Make changes to Svelte components or stores
2. Vite will hot-reload changes automatically
3. Check types with `bun run check` before committing

### Working with Rust Backend

1. Define Tauri commands in appropriate module (e.g., `src-tauri/src/commands/clipboard.rs`)
2. Export command in `src-tauri/src/commands/mod.rs`
3. Register command in `src-tauri/src/lib.rs` invoke_handler
4. Create typed wrapper in `src/lib/tauri/commands.ts`
5. Use command through repository or store

### Adding New Features

Follow the Clean Architecture layers:

1. **Domain**: Define types/interfaces in `lib/types/`
2. **Infrastructure**:
   - Implement Rust commands in `src-tauri/src/commands/`
   - Create TypeScript wrappers in `lib/tauri/commands.ts`
3. **Application**: Create/update stores in `lib/stores/`
4. **Presentation**: Build UI components and pages

### Adding Pagination to New Features

1. **Backend (Rust)**:
   ```rust
   pub fn get_items_paginated(&self, limit: i64, offset: i64) -> Result<Vec<Item>>
   pub fn count_items(&self) -> Result<i64>
   ```

2. **Commands**:
   ```rust
   #[tauri::command]
   pub fn get_items_paginated(limit: i64, offset: i64, state: State) -> Result<Vec<Item>, String>
   ```

3. **Frontend Store**:
   ```typescript
   class Store {
     items = $state<Item[]>([])
     totalItems = $state(0)
     currentPage = $state(0)
     pageSize = $state(50)
     hasMore = $state(true)

     async loadMore() { /* ... */ }
   }
   ```

4. **Component (Infinite Scroll)**:
   ```svelte
   <div onscroll={handleScroll}>
     {#if store.isLoadingMore}
       <div>Loading more...</div>
     {/if}
   </div>
   ```

## Testing Strategy

### Current Testing
- **Manual testing** on macOS
- **Type checking** with TypeScript
- **Linting** with Clippy

### Future Testing (Planned)
- **Frontend**: Vitest for unit tests, Playwright for E2E
- **Backend**: Rust `cargo test` for unit and integration tests
- **Tauri Commands**: Test through frontend E2E tests

## Build and Distribution

### Development Build
```bash
bun run tauri dev
```

### Production Build
```bash
bun run tauri build
```

Outputs platform-specific installers in `src-tauri/target/release/bundle/`:
- **macOS**: `.app` and `.dmg`
- **Windows**: `.msi` and `.exe`
- **Linux**: `.deb`, `.AppImage`

## Known Issues and Troubleshooting

### Port 1420 already in use
```bash
lsof -ti:1420 | xargs kill -9
```

### Hotkey already registered by another app
- Kill the old process: `pkill -9 global_clipboard`
- Change the hotkey in Settings
- Default fallback: App continues without hotkey

### App won't start
```bash
# Kill existing processes
pkill -9 global_clipboard

# Clear app data (WARNING: deletes all clipboard history)
rm -rf ~/Library/Application\ Support/com.dereckan.quakboard/
```

### Rust compilation errors
- Ensure Xcode Command Line Tools: `xcode-select --install`
- Update Rust: `rustup update`

### TypeScript errors
- Clear cache: `rm -rf .svelte-kit node_modules`
- Reinstall: `bun install`

## Security and Privacy

- **100% Local** - All data stored on device
- **No telemetry** - No tracking or analytics
- **No internet required** - Works offline (except URL metadata)
- **Sandboxed** - Tauri security model with explicit permissions
- **Open source** - Auditable code

### Permissions (src-tauri/capabilities/default.json)
```json
{
  "permissions": [
    "core:default",
    "opener:default",
    "notification:default",
    "notification:allow-is-permission-granted",
    "notification:allow-request-permission",
    "notification:allow-show",
    "global-shortcut:default",
    "global-shortcut:allow-is-registered",
    "global-shortcut:allow-register",
    "global-shortcut:allow-register-all",
    "global-shortcut:allow-unregister",
    "global-shortcut:allow-unregister-all"
  ]
}
```

## Performance Optimizations

1. **Server-side pagination** - Loads 50-100 items at a time
2. **Infinite scroll** - Only loads more when needed
3. **Duplicate detection** - Prevents saving same content twice
4. **Efficient queries** - Indexed database queries
5. **Background monitoring** - 500ms polling interval (non-blocking)
6. **Rust backend** - Native performance with low memory usage

## Code Quality

### Linting Rules
- **Rust**: Clippy with default rules
- **TypeScript**: ESLint with recommended rules
- **Svelte**: Svelte-check for component validation

### Code Style
- **Rust**: `cargo fmt` (rustfmt)
- **TypeScript**: Prettier
- **Commits**: Conventional commits preferred

## Future Roadmap

**Ver el archivo [ROADMAP.md](./ROADMAP.md) para el plan completo y detallado.**

### Próximos Pasos Inmediatos

#### 🐛 Bugs a Arreglar
1. **Search bar no funciona** - PRIORIDAD ALTA
   - El usuario escribe pero no ve resultados
   - Posible problema con eventos o feedback visual
   - Ver `src/lib/components/header/SearchBar.svelte`

#### 🎨 Mejoras de UI/UX (Fácil - 1-2 semanas)
- Mejorar diseño visual general
- Soporte para SVG
- Settings avanzados (límites, retención, uso de memoria)
- Soporte para imágenes copiadas

#### 📱 Features Medias (Medio - 1-2 meses)
- UI responsive para móviles y tablets
- Testing básico (Vitest + cargo test)
- Sistema de sincronización local-cloud

#### 🔐 Features Avanzadas (Difícil - 3-6 meses)
- Sistema de cuentas con Supabase
- OAuth (Google, GitHub, email/password)
- Monetización con Stripe
- Planes Free y Pro

### Roadmap de Versiones

```
v1.0 (ACTUAL) ✅
├─ Features core implementadas
└─ Local storage funcional

v1.1 (1-2 meses) 🔄
├─ Arreglar search bar
├─ Mejorar diseño
├─ Soporte imágenes/SVG
└─ Settings avanzados

v1.2 (2-3 meses) 📱
├─ UI responsive
├─ Cuentas y auth
└─ Sync con Supabase

v2.0 (4-6 meses) 💰
├─ Sistema de planes
├─ Pagos con Stripe
└─ Features Pro

v3.0+ (6-12 meses) 🚀
├─ Colaboración
├─ API pública
├─ Extensiones
└─ Apps móviles
```

## Resources

- [Tauri v2 Docs](https://v2.tauri.app/)
- [SvelteKit Docs](https://svelte.dev/docs/kit)
- [Svelte 5 Runes](https://svelte.dev/docs/svelte/what-are-runes)
- [TailwindCSS v4](https://tailwindcss.com/blog/tailwindcss-v4-beta)
- [Rust Book](https://doc.rust-lang.org/book/)
- [Project README](./README.md) - User documentation
- [Contributing Guide](./CONTRIBUTING.md) - Contribution guidelines

## Recent Major Changes

### v1.0 Implementation Summary

**Completed Features** (in order):
1. ✅ **Phase 1-4**: Basic UI, clipboard detection, database integration
2. ✅ **Pagination System**: Server-side pagination with infinite scroll
3. ✅ **Dynamic Hotkeys**: Change keyboard shortcuts without restart
4. ✅ **macOS Integration**: Hide from Dock, auto-hide on focus loss
5. ✅ **Search**: Full-text search with pagination
6. ✅ **Date Grouping**: Items organized by date ranges
7. ✅ **Code Cleanup**: Removed unused functions, applied linting

**Key Refactorings**:
- Removed `get_items()` in favor of `get_items_paginated()`
- Created `shortcuts.rs` module for hotkey management
- Fixed `$derived()` vs `$derived.by()` in search filters
- Updated all stores to use pagination
- Cleaned up commented code and unused imports

**Files Modified** (major changes):
- `src-tauri/src/shortcuts.rs` - New module
- `src-tauri/src/commands/settings.rs` - Dynamic hotkey updates
- `src-tauri/src/db/repository.rs` - Pagination methods
- `src-tauri/src/lib.rs` - App initialization with hotkeys
- `src/lib/stores/clipboard.svelte.ts` - Pagination state
- `src/lib/components/sidebar/SidebarGroup.svelte` - Date grouping
- `src/lib/components/sidebar/Sidebar.svelte` - Infinite scroll
- `src/routes/+page.svelte` - Fixed search with $derived.by()

### v1.1 Recent Implementations (Current Session)

**Completed Features**:

1. ✅ **Full-Text Search with SQLite FTS5**
   - Multi-field search across content_text, content_metadata, file_name, snippet_name, code_language
   - BM25 ranking for relevance
   - Prefix matching for partial queries
   - Phrase search with quotes
   - Debounced automatic search (300ms delay, no Enter key needed)
   - Files: `src-tauri/src/db/fts_migration.rs`, `repository.rs`

2. ✅ **Search Result Highlighting**
   - Visual highlighting of search terms in results
   - Reusable component for highlighted text
   - Files: `src/lib/utils/highlight.ts`, `src/lib/components/ui/HighlightedText.svelte`

3. ✅ **Advanced Settings Management**
   - Item limit controls with toggle and slider (100-5000 items)
   - Automatic retention policies (7, 30, 90, 180, 365 days)
   - Database statistics display (total items, favorites, snippets, DB size)
   - Manual cleanup and optimization tools
   - Files: `src-tauri/src/cleanup/mod.rs`, `src/lib/stores/settings.svelte.ts`, `src/routes/settings/+page.svelte`

4. ✅ **Cross-Platform Window Customization**
   - Borderless window with rounded corners on all platforms
   - Native drag functionality preserved
   - Auto-hide on focus loss (with platform-specific delay for Windows)
   - macOS: Uses `macOSPrivateApi: true` + `decorations: true` + `titleBarStyle: "Overlay"`
   - Windows: Same config works with optional delay for drag detection
   - Configuration: `tauri.conf.json` with `decorations: true`, `titleBarStyle: "Overlay"`, `hiddenTitle: true`

**Key Technical Decisions**:

- **Search Implementation**: Chose FTS5 over LIKE queries for better performance and ranking
- **Window Customization**: Chose `decorations: true` with `macOSPrivateApi: true` instead of platform-specific native code (cocoa/Win32) for simplicity
- **Auto-hide behavior**: Platform-specific delays added for Windows to prevent hide during window drag

**Database Changes**:
- Added FTS5 virtual table `clipboard_items_fts` with triggers for automatic sync
- Cleanup functions respect favorites (never delete favorited items)
- Database vacuum for optimization

**Configuration Files Updated**:
- `tauri.conf.json` - Window configuration for cross-platform support
- `Cargo.toml` - No native dependencies needed (cocoa/Win32 not required)
- `src/lib/types/settings.ts` - New settings fields for cleanup and retention

**Removed/Cleaned Code**:
- Old LIKE-based search queries replaced with FTS5
- Attempted cocoa/Win32 native window code removed (not needed with macOSPrivateApi)
- Unused imports and deprecated functions cleaned up

## Behavior 

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.