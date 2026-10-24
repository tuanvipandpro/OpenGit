export function directoryError(value: string): string | null {
  const stem = value.split('.')[0].replace(/ +$/, '');
  if (!value || value.length > 200 || value === '.' || value === '..' || /[. ]$/.test(value) || /[\/\\:<>"|?*\p{Cc}]/u.test(value) || /^(?:CON|PRN|AUX|NUL|CLOCK\$|(?:COM|LPT)[1-9¹²³])$/i.test(stem)) return 'Choose a valid Windows folder name (1–200 characters), without reserved names or trailing dots or spaces.';
  return null;
}
export function sourceError(value: string): string | null {
  const invalid = 'Enter an HTTPS or SSH repository address without passwords, tokens, query strings or fragments. Use the folder picker for local repositories.';
  if (!value || new Blob([value]).size > 4096 || value.startsWith('-') || /[\p{Cc}\\?#]/u.test(value)) return invalid;
  if (value.includes('://')) {
    try { const url = new URL(value); if (!['https:', 'ssh:'].includes(url.protocol) || !url.hostname || url.password || (url.protocol === 'https:' && url.username) || url.search || url.hash || (url.protocol === 'ssh:' && (!/^[a-zA-Z0-9._-]*$/.test(url.username) || url.username.startsWith('-') || url.hostname.startsWith('-')))) return invalid; }
    catch { return invalid; }
  } else {
    if (value.includes('::')) return invalid;
    const match = /^(?:([a-zA-Z0-9._-]+)@)?([a-zA-Z0-9._-]{2,}):(.+)$/.exec(value);
    if (!match || match[1]?.startsWith('-') || match[2].startsWith('-')) return invalid;
  }
  return null;
}
