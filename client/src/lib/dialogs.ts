import type { ReactNode } from 'react'
import { create } from 'zustand'
import type { ChannelKind } from './api'

/** Bereiche der Nutzer-Einstellungen. */
export type SettingsTab =
  | 'account'
  | 'security'
  | 'profile'
  | 'sessions'
  | 'privacy'
  | 'storage'
  | 'voice'
  | 'sounds'
  | 'notifications'
  | 'appearance'
  | 'desktop'
  | 'advanced'
  | 'developer'

export type CommunityTab =
  | 'overview'
  | 'appearance'
  | 'emojis'
  | 'roles'
  | 'members'
  | 'bans'
  | 'invites'
  | 'automod'
  | 'audit'
  | 'danger'
export type ChannelTab = 'overview' | 'permissions' | 'webhooks'

/** Bestätigung einer destruktiven Aktion. Fehler der Aktion werden im Dialog angezeigt. */
export interface ConfirmSpec {
  title: string
  message: ReactNode
  confirmLabel: string
  /** Rot statt Akzentfarbe (Löschen, Entfernen, Verlassen). Standard: `true`. */
  danger?: boolean
  action: () => Promise<unknown> | unknown
}

/** Global geöffnete Dialoge (aus Kontextmenüs, Dropdowns, Tastenkürzeln). */
export type DialogSpec =
  | ({ kind: 'confirm' } & ConfirmSpec)
  | { kind: 'nickname'; communityId: string; userId: string }
  | { kind: 'ban'; communityId: string; userId: string; name: string }
  | { kind: 'invite'; communityId: string }
  | { kind: 'createChannel'; communityId: string; channelKind?: ChannelKind; categoryId?: string | null }
  /** Kategorie anlegen (ohne `categoryId`) oder umbenennen. */
  | { kind: 'category'; communityId: string; categoryId?: string }
  | { kind: 'communitySettings'; communityId: string; tab?: CommunityTab }
  | { kind: 'channelSettings'; communityId: string; channelId: string; tab?: ChannelTab }
  | { kind: 'settings'; tab?: SettingsTab }
  | { kind: 'groupDm'; dmId: string }
  | { kind: 'statusText' }
  | { kind: 'newDm' }
  | { kind: 'createGroupDm' }
  | { kind: 'addToDm'; dmId: string }

type Entry = DialogSpec & { id: number }

interface DialogStore {
  /** Stapel: der letzte liegt oben (z. B. Bestätigung über den Einstellungen). */
  stack: Entry[]
  open: (spec: DialogSpec) => void
  close: (id: number) => void
}

let nextId = 1

export const useDialogs = create<DialogStore>()((set) => ({
  stack: [],
  open(spec) {
    // Dieselbe Ebene nicht doppelt öffnen (z. B. zweimal Einstellungen per Tastenkürzel).
    set((s) => {
      const same = s.stack.find((d) => d.kind === spec.kind && spec.kind !== 'confirm')
      const rest = same ? s.stack.filter((d) => d !== same) : s.stack
      return { stack: [...rest, { ...spec, id: nextId++ }] }
    })
  },
  close(id) {
    set((s) => ({ stack: s.stack.filter((d) => d.id !== id) }))
  },
}))

export function openDialog(spec: DialogSpec) {
  useDialogs.getState().open(spec)
}

export function confirmAction(spec: ConfirmSpec) {
  useDialogs.getState().open({ kind: 'confirm', ...spec })
}
