import { useCallback, useRef, useState } from 'react'
import type { ClipboardEvent, DragEvent } from 'react'
import { useStore } from '../store'
import { useUploads } from './uploads'

/** Dateien zum Composer eines Kanals hinzufügen (prüft Anzahl und Größenlimit der Instanz). */
export function useAddFiles(channelId: string) {
  const add = useUploads((s) => s.add)
  const maxBytes = useStore((s) => s.instance?.max_upload_bytes)
  return useCallback((files: File[]) => files.length > 0 && add(channelId, files, maxBytes), [add, channelId, maxBytes])
}

/** Dateien aus der Zwischenablage (z. B. Screenshots) übernehmen. */
export function usePasteFiles(channelId: string, enabled: boolean) {
  const addFiles = useAddFiles(channelId)
  return useCallback(
    (e: ClipboardEvent) => {
      if (!enabled) return
      const files = Array.from(e.clipboardData.files)
      if (files.length === 0) return
      e.preventDefault()
      addFiles(files)
    },
    [addFiles, enabled],
  )
}

/** Drag & Drop auf eine Fläche; liefert Props für das Element und ob gerade gezogen wird. */
export function useFileDrop(channelId: string, enabled: boolean) {
  const addFiles = useAddFiles(channelId)
  const [dragging, setDragging] = useState(false)
  const depth = useRef(0)

  const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer.types).includes('Files')

  const props = {
    onDragEnter: (e: DragEvent) => {
      if (!enabled || !hasFiles(e)) return
      e.preventDefault()
      depth.current++
      setDragging(true)
    },
    onDragOver: (e: DragEvent) => {
      if (!enabled || !hasFiles(e)) return
      e.preventDefault()
      e.dataTransfer.dropEffect = 'copy'
    },
    onDragLeave: (e: DragEvent) => {
      if (!enabled || !hasFiles(e)) return
      depth.current = Math.max(0, depth.current - 1)
      if (depth.current === 0) setDragging(false)
    },
    onDrop: (e: DragEvent) => {
      if (!enabled || !hasFiles(e)) return
      e.preventDefault()
      depth.current = 0
      setDragging(false)
      addFiles(Array.from(e.dataTransfer.files))
    },
  }
  return { dragging, props }
}
