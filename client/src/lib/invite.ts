/** Akzeptiert einen reinen Code oder einen kompletten Link `…/invite/{code}`. */
export function extractInviteCode(input: string): string {
  const trimmed = input.trim()
  const m = trimmed.match(/\/invite\/([^/?#\s]+)/)
  return m ? decodeURIComponent(m[1]) : trimmed
}
