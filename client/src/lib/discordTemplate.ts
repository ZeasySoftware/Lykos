// Discord-Servervorlagen: Code aus einem eingefügten Link lesen (der Server prüft ihn erneut).

const HOSTS = new Set(['discord.new', 'discord.com', 'www.discord.com', 'discordapp.com', 'www.discordapp.com', 'discord.gg'])
const CODE = /^[A-Za-z0-9-]{2,64}$/

/**
 * Vorlagen-Code aus Code oder Link (`https://discord.new/CODE`, `https://discord.com/template/CODE`,
 * auch `discordapp.com`/`discord.gg`, mit oder ohne `https://`). Alles andere → `null`.
 */
export function extractTemplateCode(input: string): string | null {
  const text = input.trim()
  if (CODE.test(text)) return text
  let url: URL
  try {
    url = new URL(text.includes('://') ? text : `https://${text}`)
  } catch {
    return null
  }
  if ((url.protocol !== 'https:' && url.protocol !== 'http:') || url.port || url.username || url.password) return null
  const host = url.hostname.toLowerCase()
  if (!HOSTS.has(host)) return null
  const parts = url.pathname.split('/').filter(Boolean)
  let code: string | undefined
  if (host === 'discord.new' && parts.length === 1) code = parts[0]
  else if (parts.length === 2 && (parts[0] === 'template' || parts[0] === 'templates')) code = parts[1]
  return code && CODE.test(code) ? code : null
}
