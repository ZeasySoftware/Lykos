import { useCallback, useEffect, useLayoutEffect } from 'react'
import type { RefObject } from 'react'

/**
 * Textfeld wächst mit seinem Inhalt (auch mit umbrochenen Zeilen) bis zur CSS-`max-height`;
 * erst darüber erscheint eine Scrollleiste. Passt die Höhe auch an, wenn sich die Breite ändert
 * (schmales Fenster, Seitenleisten ein-/ausgeblendet).
 */
export function useAutoGrow(ref: RefObject<HTMLTextAreaElement | null>, value: string) {
  const fit = useCallback(() => {
    const el = ref.current
    if (!el) return
    el.style.overflowY = 'hidden'
    el.style.height = 'auto'
    const style = getComputedStyle(el)
    const borders = parseFloat(style.borderTopWidth) + parseFloat(style.borderBottomWidth)
    const needed = el.scrollHeight + (style.boxSizing === 'border-box' ? borders : 0)
    const max = parseFloat(style.maxHeight)
    const limit = Number.isFinite(max) ? max : Infinity
    el.style.height = `${Math.min(needed, limit)}px`
    el.style.overflowY = needed > limit ? 'auto' : 'hidden'
  }, [ref])

  useLayoutEffect(fit, [fit, value])

  useEffect(() => {
    const el = ref.current
    if (!el || typeof ResizeObserver === 'undefined') return
    // Nur auf Breitenänderungen reagieren – die Höhe setzen wir selbst.
    let width = el.clientWidth
    const observer = new ResizeObserver(() => {
      if (el.clientWidth === width) return
      width = el.clientWidth
      fit()
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [ref, fit])
}
