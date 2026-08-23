/**
 * Monochrome 24x24 line icons. Every glyph inherits `currentColor` and a single
 * stroke weight, so colour is decided by CSS at the call site rather than baked
 * into the artwork. Each entry is the inner markup of the shared <svg> wrapper.
 */
const P = {
  /* ---- toolbar ---- */
  'add-file': `
    <path d="M14.5 2.5H6.5a2 2 0 0 0-2 2v15a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V7.5z"/>
    <path d="M14.5 2.5v5h5"/>
    <path d="M9.5 15.5h5M12 13v5"/>`,

  'add-url': `
    <path d="M9.5 17h-2a5 5 0 0 1 0-10h2"/>
    <path d="M14.5 7h2a5 5 0 0 1 0 10h-2"/>
    <path d="M8 12h8"/>`,

  create: `
    <rect x="3" y="3" width="18" height="18" rx="4.5"/>
    <path d="M12 8.5v7M8.5 12h7"/>`,

  remove: `
    <path d="M4 6.5h16"/>
    <path d="M9.5 6.5v-2a1.5 1.5 0 0 1 1.5-1.5h2a1.5 1.5 0 0 1 1.5 1.5v2"/>
    <path d="M18 6.5v13a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2v-13"/>
    <path d="M10 11v6M14 11v6"/>`,

  start: `<path d="M7.5 4.8 19 12 7.5 19.2z" fill="currentColor" stroke-linejoin="round"/>`,

  pause: `
    <rect x="7" y="4.5" width="3.6" height="15" rx="1.3" fill="currentColor" stroke="none"/>
    <rect x="13.4" y="4.5" width="3.6" height="15" rx="1.3" fill="currentColor" stroke="none"/>`,

  stop: `<rect x="5.5" y="5.5" width="13" height="13" rx="2.6" fill="currentColor" stroke="none"/>`,

  'queue-up': `<path d="M12 19.5v-14M6 11.5 12 5.5l6 6"/>`,

  'queue-down': `<path d="M12 4.5v14M6 12.5l6 6 6-6"/>`,

  'alt-speed': `
    <path d="M3.5 18.5a10 10 0 1 1 17 0"/>
    <path d="m12 14.5 4.5-4.5"/>
    <circle cx="12" cy="14.5" r="1.4" fill="currentColor" stroke="none"/>`,

  preferences: `
    <path d="M3.5 6H14M18 6h2.5"/>
    <path d="M3.5 12H7M11 12h9.5"/>
    <path d="M3.5 18H13M17 18h3.5"/>
    <circle cx="16" cy="6" r="2"/>
    <circle cx="9" cy="12" r="2"/>
    <circle cx="15" cy="18" r="2"/>`,

  search: `<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4.5 4.5"/>`,

  /* ---- sidebar ---- */
  torrents: `
    <path d="M21.5 12.5h-5l-2 3h-5l-2-3h-5"/>
    <path d="M5.9 5.1 2.5 12v6a2 2 0 0 0 2 2h15a2 2 0 0 0 2-2v-6l-3.4-6.9a2 2 0 0 0-1.8-1.1H7.7a2 2 0 0 0-1.8 1.1z"/>`,

  downloading: `<path d="M12 4.5v11M7.5 11l4.5 4.5 4.5-4.5"/><path d="M4.5 19.5h15"/>`,

  seeding: `<path d="M12 19.5v-11M7.5 13 12 8.5l4.5 4.5"/><path d="M4.5 4.5h15"/>`,

  completed: `<circle cx="12" cy="12" r="9"/><path d="m8 12.2 2.8 2.8L16 9.5"/>`,

  active: `<path d="M13 2.5 4.5 13.5H12l-1 8 8.5-11H13z"/>`,

  inactive: `<circle cx="12" cy="12" r="9"/><path d="M10 9.2v5.6M14 9.2v5.6"/>`,

  label: `
    <path d="M12.6 2.6A2 2 0 0 0 11.2 2H4a2 2 0 0 0-2 2v7.2a2 2 0 0 0 .6 1.4l8.7 8.7a2.4 2.4 0 0 0 3.4 0l6.6-6.6a2.4 2.4 0 0 0 0-3.4z"/>
    <circle cx="7.4" cy="7.4" r="1.3"/>`,

  feeds: `
    <path d="M4 11a9 9 0 0 1 9 9"/>
    <path d="M4 4a16 16 0 0 1 16 16"/>
    <circle cx="5" cy="19" r="1.6" fill="currentColor" stroke="none"/>`,

  error: `
    <circle cx="12" cy="12" r="9"/>
    <path d="M12 7.4v5.4"/>
    <circle cx="12" cy="16.4" r="1.1" fill="currentColor" stroke="none"/>`,

  /* ---- detail tabs ---- */
  info: `
    <circle cx="12" cy="12" r="9"/>
    <path d="M12 16.4v-4.8"/>
    <circle cx="12" cy="8" r="1.1" fill="currentColor" stroke="none"/>`,

  tracker: `
    <circle cx="12" cy="12" r="8.5"/>
    <circle cx="12" cy="12" r="4"/>
    <circle cx="12" cy="12" r="1.3" fill="currentColor" stroke="none"/>`,

  peers: `
    <path d="M15.5 20.5v-1.8a3.6 3.6 0 0 0-3.6-3.6H6.6A3.6 3.6 0 0 0 3 18.7v1.8"/>
    <circle cx="9.2" cy="7.6" r="3.6"/>
    <path d="M21 20.5v-1.8a3.6 3.6 0 0 0-2.7-3.5"/>
    <path d="M15.6 4.1a3.6 3.6 0 0 1 0 7"/>`,

  pieces: `
    <rect x="3" y="3" width="7.6" height="7.6" rx="1.6"/>
    <rect x="13.4" y="3" width="7.6" height="7.6" rx="1.6"/>
    <rect x="3" y="13.4" width="7.6" height="7.6" rx="1.6"/>
    <rect x="13.4" y="13.4" width="7.6" height="7.6" rx="1.6"/>`,

  files: `
    <path d="M14.5 2.5H6.5a2 2 0 0 0-2 2v15a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V7.5z"/>
    <path d="M14.5 2.5v5h5"/>
    <path d="M8.5 12.5h7M8.5 16.5h4.5"/>`,

  speed: `<path d="M21.5 12h-4l-3 8.5-5-17-3 8.5h-4"/>`,

  logger: `<path d="m4.5 17 6-5-6-5"/><path d="M12.5 19h7"/>`,

  /* ---- status bar & misc ---- */
  down: `<path d="M12 4.5v13M6.5 12l5.5 5.5 5.5-5.5"/>`,

  up: `<path d="M12 19.5v-13M6.5 12 12 6.5l5.5 5.5"/>`,

  folder: `<path d="M20 20.5H4a2 2 0 0 1-2-2v-13a2 2 0 0 1 2-2h4.6l2.4 3H20a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2z"/>`,

  magnet: `
    <path d="M3.5 4.5h4v8a4.5 4.5 0 0 0 9 0v-8h4v8a8.5 8.5 0 0 1-17 0z"/>
    <path d="M3.5 8.5h4M16.5 8.5h4"/>`,

  /* The app mark, in the same proportions build/logo.svg and the icon carry:
     two bars with three lighter steps walking the diagonal between them. The
     per-path widths override the set's shared stroke weight on purpose. */
  logo: `
    <path d="M5.8 5.2h12.4" stroke-width="3.4"/>
    <path d="M12.1 8.95h4" stroke-width="2"/>
    <path d="M10 12h4" stroke-width="2"/>
    <path d="M7.9 15.05h4" stroke-width="2"/>
    <path d="M5.8 18.8h12.4" stroke-width="3.4"/>`
}

export function icon (name, size, cls) {
  const body = P[name]
  if (!body) return ''
  const dim = size ? ` width="${size}" height="${size}"` : ''
  const klass = cls ? ` class="${cls}"` : ''
  return `<svg viewBox="0 0 24 24"${dim}${klass} fill="none" stroke="currentColor" stroke-width="1.75" ` +
         `stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`
}

export const ICON_NAMES = Object.keys(P)
