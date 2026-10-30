// Gesammeltes Neuladen nach PERMISSIONS_CHANGED/ROLE_DELETE. Ausführen mit `npm test`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { createRefreshBatch, type RefreshBatchOptions } from './refreshBatch.ts'

/** Künstliche Uhr mit einem Timer (mehr braucht der Batcher nie gleichzeitig). */
function fakeClock() {
  let time = 0
  let timer: { fn: () => void; at: number } | null = null
  const options: RefreshBatchOptions = {
    now: () => time,
    setTimer: (fn, ms) => (timer = { fn, at: time + ms }),
    clearTimer: (handle) => {
      if (handle === timer) timer = null
    },
  }
  return {
    options,
    /** Zeit vorspulen und fällige Timer ausführen. */
    advance(ms: number) {
      time += ms
      if (timer && timer.at <= time) {
        const { fn } = timer
        timer = null
        fn()
      }
    },
    get dueAt() {
      return timer?.at ?? null
    },
  }
}

test('Zufallsverzögerung zwischen min und max', () => {
  for (const [random, expected] of [
    [0, 300],
    [0.5, 1150],
    [0.999, 1998.3],
  ] as const) {
    const clock = fakeClock()
    const runs: string[][] = []
    const batch = createRefreshBatch((ids) => runs.push(ids), { ...clock.options, random: () => random })
    batch.add('c1')
    assert.equal(clock.dueAt, expected)
    clock.advance(expected - 1)
    assert.equal(runs.length, 0)
    clock.advance(1)
    assert.deepEqual(runs, [['c1']])
  }
})

test('mehrere Events werden zu einem Abruf je ID zusammengefasst', () => {
  const clock = fakeClock()
  const runs: string[][] = []
  const batch = createRefreshBatch((ids) => runs.push(ids), { ...clock.options, random: () => 0.5 })
  batch.add('c1')
  clock.advance(100)
  batch.add('c1')
  batch.add('c2')
  clock.advance(100)
  batch.add('c1')
  assert.deepEqual(batch.pending(), ['c1', 'c2'])
  clock.advance(2000)
  assert.deepEqual(runs, [['c1', 'c2']])
  assert.deepEqual(batch.pending(), [])
})

test('späte Events schieben den Abruf um die Ruhezeit hinaus, höchstens bis maxWait', () => {
  const clock = fakeClock()
  const runs: string[][] = []
  const batch = createRefreshBatch((ids) => runs.push(ids), { ...clock.options, random: () => 0 })
  batch.add('c1') // fällig bei 300
  clock.advance(250)
  batch.add('c1') // Ruhezeit → 550
  assert.equal(clock.dueAt, 550)
  // Dauerfeuer alle 200 ms: spätestens nach 5 s wird trotzdem geladen.
  for (let i = 0; i < 30 && runs.length === 0; i++) {
    clock.advance(200)
    if (runs.length === 0) batch.add('c1')
  }
  assert.equal(runs.length, 1)
  assert.equal(clock.dueAt, null)
})

test('nach dem Abruf beginnt eine neue Sammlung; flush und cancel', () => {
  const clock = fakeClock()
  const runs: string[][] = []
  const batch = createRefreshBatch((ids) => runs.push(ids), { ...clock.options, random: () => 0 })
  batch.add('a')
  clock.advance(300)
  batch.add('b')
  batch.flush()
  assert.deepEqual(runs, [['a'], ['b']])
  assert.equal(clock.dueAt, null)

  batch.add('c')
  batch.cancel()
  clock.advance(10_000)
  assert.deepEqual(runs, [['a'], ['b']])
  batch.flush()
  assert.deepEqual(runs, [['a'], ['b']])
})
