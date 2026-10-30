/**
 * Scrollleisten nur beim Scrollen zeigen: Das gerade scrollende Element bekommt `is-scrolling`, das
 * `SCROLLBAR_HIDE_MS` nach der letzten Bewegung wieder verschwindet. Aussehen in `styles/base.css`.
 */
export const SCROLLBAR_HIDE_MS = 900

const timers = new WeakMap<Element, number>()

function onScroll(e: Event) {
  const el = e.target instanceof Element ? e.target : document.scrollingElement
  if (!el) return
  el.classList.add('is-scrolling')
  const pending = timers.get(el)
  if (pending !== undefined) window.clearTimeout(pending)
  timers.set(
    el,
    window.setTimeout(() => {
      el.classList.remove('is-scrolling')
      timers.delete(el)
    }, SCROLLBAR_HIDE_MS),
  )
}

/** Einmal beim Start aufrufen (Scroll-Ereignisse blubbern nicht – daher in der Capture-Phase). */
export function installScrollbarAutoHide() {
  document.addEventListener('scroll', onScroll, { capture: true, passive: true })
}
