import { useRef } from 'react'
import type { TextareaHTMLAttributes } from 'react'
import { useAutoGrow } from '../lib/autoGrow'

/** Textfeld, das mit dem Inhalt wächst; Scrollleiste erst ab der CSS-`max-height`. */
export function AutoGrowTextarea(props: TextareaHTMLAttributes<HTMLTextAreaElement> & { value: string }) {
  const ref = useRef<HTMLTextAreaElement>(null)
  useAutoGrow(ref, props.value)
  return <textarea {...props} ref={ref} rows={1} />
}
