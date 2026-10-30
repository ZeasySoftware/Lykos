/**
 * Öffentliche Issue-Seite zum Quellcode der Instanz (`/instance` → `source_url`), für den Hinweis „Lieber
 * öffentlich melden?“ im Feedback-Fenster. Nur für GitHub-Repos (`https://github.com/<owner>/<repo>`), sonst `null`
 * – bei anderen Hostern kennen wir die Adresse für neue Issues nicht verlässlich.
 */
export function publicIssueUrl(sourceUrl: string | null | undefined): string | null {
  if (!sourceUrl) return null
  let url: URL
  try {
    url = new URL(sourceUrl)
  } catch {
    return null
  }
  if (url.protocol !== 'https:' || url.hostname !== 'github.com') return null
  const [owner, repo] = url.pathname.split('/').filter(Boolean)
  if (!owner || !repo) return null
  return `https://github.com/${owner}/${repo.replace(/\.git$/, '')}/issues/new/choose`
}
