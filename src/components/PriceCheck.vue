<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'

const emit = defineEmits<{ interact: []; close: [] }>()

type GameMode = 'poe1' | 'poe2'
type BuyoutType = 'iob' | 'inperson' | 'both'
type Rarity = 'normal' | 'magic' | 'rare' | 'unique' | 'currency' | 'gem' | 'divination'

interface League { id: string; text: string }

interface Affix {
  id: string; text: string; value: number
  min: number | null; max: number | null; enabled: boolean
  stat_id: string | null
  mod_group: string
}
interface BaseStat {
  id: string; label: string; value: number
  min: number | null; max: number | null; enabled: boolean
}
interface Currency { id: string; label: string }
interface Result {
  price: number; currency: string; seller: string
  status: 'online' | 'afk' | 'offline'
}
interface ItemData {
  name: string; base_type: string; rarity: string
  item_level: number; influence: string; game_mode: string
  item_class: string; corrupted: boolean
  base_stats: Array<{ id: string; label: string; value: number }>
  mods: Array<{ text: string; value: number; stat_id: string | null; mod_group: string }>
}

const gameMode = ref<GameMode>('poe1')
const buyoutType = ref<BuyoutType>('iob')

const FALLBACK_LEAGUES: League[] = [
  { id: 'Standard',  text: 'Standard' },
  { id: 'Hardcore',  text: 'Hardcore' },
]
const leagues        = ref<League[]>([])
const selectedLeague = ref('')
const loadingLeagues = ref(false)

async function loadLeagues() {
  loadingLeagues.value = true
  try {
    const result = await invoke<League[]>('fetch_leagues', { gameMode: gameMode.value })
    leagues.value = result.length ? result : FALLBACK_LEAGUES
  } catch {
    leagues.value = FALLBACK_LEAGUES
  } finally {
    loadingLeagues.value = false
  }
  const saved = localStorage.getItem(`${gameMode.value}_league`)
  const match = leagues.value.find(l => l.id === saved)
  selectedLeague.value = match ? match.id : (leagues.value[0]?.id ?? '')
}

function onLeagueChange(e: Event) {
  const id = (e.target as HTMLSelectElement).value
  selectedLeague.value = id
  localStorage.setItem(`${gameMode.value}_league`, id)
  handleInteract()
}

function onCurrencyChange() {
  localStorage.setItem(`${gameMode.value}_currency`, selectedCurrency.value)
  handleInteract()
}

watch(gameMode, loadLeagues)

const POE1_CURRENCIES: Currency[] = [
  { id: 'chaos',   label: 'Chaos Orb' },
  { id: 'divine',  label: 'Divine Orb' },
  { id: 'exalted', label: 'Exalted Orb' },
  { id: 'mirror',  label: 'Mirror of Kalandra' },
  { id: 'alt',     label: 'Orb of Alteration' },
  { id: 'fuse',    label: 'Orb of Fusing' },
  { id: 'chrome',  label: 'Chromatic Orb' },
]
const POE2_CURRENCIES: Currency[] = [
  { id: 'exalted', label: 'Exalted Orb' },
  { id: 'divine',  label: 'Divine Orb' },
  { id: 'chaos',   label: 'Chaos Orb' },
  { id: 'mirror',  label: 'Mirror of Kalandra' },
  { id: 'alt',     label: 'Orb of Alteration' },
]

const currencies = computed(() =>
  gameMode.value === 'poe1' ? POE1_CURRENCIES : POE2_CURRENCIES
)
function savedCurrency(mode: GameMode): string {
  const saved = localStorage.getItem(`${mode}_currency`)
  const list = mode === 'poe1' ? POE1_CURRENCIES : POE2_CURRENCIES
  return list.find(c => c.id === saved)?.id ?? (mode === 'poe1' ? 'chaos' : 'exalted')
}

const selectedCurrency = ref(savedCurrency('poe1'))

const showSettings     = ref(false)
const sessionInput     = ref('')
const sessionIsSet     = ref(false)
const detectStatus     = ref<'idle' | 'busy' | 'ok' | 'err'>('idle')
const detectError      = ref('')

function toggleSettings() {
  showSettings.value = !showSettings.value
  if (showSettings.value) detectStatus.value = 'idle'
  handleInteract()
}

async function autoDetect() {
  detectStatus.value = 'busy'
  detectError.value  = ''
  try {
    await invoke<string>('read_browser_session')
    // read_browser_session auto-saves — never put the value in the input
    sessionIsSet.value = true
    detectStatus.value = 'ok'
  } catch (e) {
    detectStatus.value = 'err'
    detectError.value  = String(e)
  }
}

async function saveSession() {
  const id = sessionInput.value.trim()
  if (!id) return  // blank = keep existing token, don't clear
  await invoke('set_session_id', { id })
  sessionIsSet.value = true
  sessionInput.value = ''  // clear after save — never leave token in field
  showSettings.value = false
}

const searching = ref(false)
const searchError = ref('')
const lastTradeUrl = ref('')

async function runSearch() {
  const filters = item.value.affixes
    .filter(a => a.enabled && a.stat_id)
    .map(a => ({
      stat_id: a.stat_id!,
      min: a.min != null ? a.min : undefined,
      max: a.max != null ? a.max : undefined,
    }))

  const baseFilters = [
    ...item.value.baseStats
      .filter(s => s.enabled)
      .map(s => ({
        stat_id: s.id,
        min: s.min != null ? s.min : undefined,
        max: s.max != null ? s.max : undefined,
      })),
    ...(item.value.ilvlEnabled && item.value.itemLevel > 0
      ? [{ stat_id: 'ilvl', min: item.value.ilvlMin, max: undefined }]
      : []),
  ]

  const corruptedArg = (item.value.corrupted && item.value.corruptedFilter) ? true : null
  const itemNameArg  = item.value.rarity === 'unique' ? item.value.name : ''

  const hasAnyFilter = filters.length > 0 || baseFilters.length > 0
    || item.value.typeEnabled || corruptedArg !== null || !!itemNameArg
  if (!hasAnyFilter) return

  searching.value = true
  searchError.value = ''
  results.value = []
  handleInteract()

  try {
    const res = await invoke<{ results: Result[]; trade_url: string }>('trade_search', {
      league: selectedLeague.value,
      gameMode: gameMode.value,
      currency: selectedCurrency.value,
      buyoutType: buyoutType.value,
      itemClass: item.value.itemClass,
      itemName: itemNameArg,
      itemType: item.value.typeEnabled ? item.value.baseType : '',
      corrupted: corruptedArg,
      filters,
      baseFilters,
    })
    results.value = res.results
    lastTradeUrl.value = res.trade_url
  } catch (e) {
    searchError.value = String(e)
  } finally {
    searching.value = false
  }
}

// ── Pseudo-stats ─────────────────────────────────────────────────────────────
interface PseudoContributor { pattern: string; weight: number }
interface PseudoDef { id: string; label: string; contributors: PseudoContributor[] }

const PSEUDO_DEFS: PseudoDef[] = [
  { id: 'pseudo.pseudo_total_life',
    label: 'Total Life',
    contributors: [
      { pattern: '+# to maximum life', weight: 1 },
    ] },
  { id: 'pseudo.pseudo_total_mana',
    label: 'Total Mana',
    contributors: [
      { pattern: '+# to maximum mana', weight: 1 },
    ] },
  { id: 'pseudo.pseudo_total_energy_shield',
    label: 'Total Energy Shield',
    contributors: [
      { pattern: '+# to maximum energy shield', weight: 1 },
    ] },
  { id: 'pseudo.pseudo_total_elemental_resistance',
    label: 'Total Elemental Res',
    contributors: [
      { pattern: '+#% to fire resistance',           weight: 1 },
      { pattern: '+#% to cold resistance',           weight: 1 },
      { pattern: '+#% to lightning resistance',      weight: 1 },
      { pattern: '+#% to all elemental resistances', weight: 3 },
      { pattern: '+#% to all resistances',           weight: 3 },
      { pattern: '+#% to fire and cold resistances', weight: 2 },
      { pattern: '+#% to fire and lightning resistances', weight: 2 },
      { pattern: '+#% to cold and lightning resistances', weight: 2 },
    ] },
]

function computePseudoStats(
  mods: ItemData['mods'],
  filtersOn: boolean
): Affix[] {
  return PSEUDO_DEFS.flatMap(def => {
    const value = def.contributors.reduce((sum, c) => {
      return sum + mods
        .filter(m => m.text.toLowerCase() === c.pattern)
        .reduce((s, m) => s + m.value * c.weight, 0)
    }, 0)
    if (value === 0) return []
    return [{
      id: def.id,
      text: def.label,
      value,
      min: Math.max(1, Math.floor(value * 0.8)),
      max: null,
      enabled: filtersOn,
      stat_id: def.id,
      mod_group: 'pseudo',
    } as Affix]
  })
}

const item = ref({
  name: '—',
  rarity: 'normal' as Rarity,
  baseType: 'Hover an item, then press Alt+D',
  itemLevel: 0,
  itemClass: '',
  influence: '',
  corrupted: false,
  corruptedFilter: false,
  affixes: [] as Affix[],
  baseStats: [] as BaseStat[],
  typeEnabled: false,
  ilvlEnabled: false,
  ilvlMin: 0,
})

const results = ref<Result[]>([])

const rarityClass = computed(() => `rarity-${item.value.rarity}`)

const corruptedAffixes = computed(() => item.value.affixes.filter(a => a.mod_group === 'corrupted'))
const implicitAffixes  = computed(() => item.value.affixes.filter(a => a.mod_group === 'implicit' || a.mod_group === 'enchant'))
const pseudoAffixes    = computed(() => item.value.affixes.filter(a => a.mod_group === 'pseudo'))
const explicitAffixes  = computed(() => item.value.affixes.filter(
  a => a.mod_group !== 'corrupted' && a.mod_group !== 'implicit' && a.mod_group !== 'enchant' && a.mod_group !== 'pseudo'
))

const medianPrice = computed(() => {
  if (!results.value.length) return null
  const prices = [...results.value].map(r => r.price).sort((a, b) => a - b)
  const mid = Math.floor(prices.length / 2)
  return prices.length % 2 !== 0
    ? prices[mid]
    : Math.round((prices[mid - 1] + prices[mid]) / 2)
})
const medianCurrency = computed(() => results.value[0]?.currency ?? 'c')

function setGameMode(mode: GameMode) {
  selectedCurrency.value = savedCurrency(mode)
  gameMode.value = mode  // triggers watch(gameMode, loadLeagues)
}
function formatAffix(text: string, value: number) {
  return text.replace('#', String(value))
}
function handleInteract() { emit('interact') }
function onRangeInputFocus() { handleInteract(); invoke('set_overlay_keyboard', { enabled: true }) }
function onRangeInputBlur()  { invoke('set_overlay_keyboard', { enabled: false }) }

// ── Drag to reposition ───────────────────────────────────────────────────────
let isDragging = false

function startDrag(e: MouseEvent) {
  if (e.button !== 0) return
  isDragging = true
  document.documentElement.requestPointerLock()
  emit('interact')
}
function onDragMove(e: MouseEvent) {
  if (!isDragging || !document.pointerLockElement) return
  const dx = Math.round(e.movementX), dy = Math.round(e.movementY)
  if (dx !== 0 || dy !== 0) invoke('move_overlay', { dx, dy })
}
function stopDrag() {
  if (isDragging && document.pointerLockElement) document.exitPointerLock()
}
function onPointerLockChange() {
  if (!document.pointerLockElement && isDragging) {
    isDragging = false
    invoke('save_overlay_position')
  }
}

// ── Lifecycle ────────────────────────────────────────────────────────────────
const unlisteners: UnlistenFn[] = []

onMounted(async () => {
  document.addEventListener('mousemove', onDragMove)
  document.addEventListener('mouseup', stopDrag)
  document.addEventListener('pointerlockchange', onPointerLockChange)

  loadLeagues()

  invoke<string>('get_session_id').then(async id => {
    if (id) {
      sessionIsSet.value = true
      return
    }
    // No saved session — silently try Firefox on first run
    try {
      await invoke<string>('read_browser_session')
      sessionIsSet.value = true
    } catch {
      // Not found — user can configure via settings
    }
  })

  unlisteners.push(await listen('search-started', () => {
    searching.value = true
    results.value = []
    lastTradeUrl.value = ''
  }))

  unlisteners.push(await listen<ItemData>('item-data', ({ payload }) => {
    searching.value = false
    results.value = []
    searchError.value = ''
    const newMode = payload.game_mode as GameMode
    if (newMode !== gameMode.value) setGameMode(newMode)

    const rarity = payload.rarity as Rarity
    // For unique/currency/gem: auto-search with filters off so the initial search
    // uses just name or type. User can enable individual filters and re-search.
    const autoSearch = rarity === 'unique' || rarity === 'currency' || rarity === 'gem'
    const filtersOn  = !autoSearch

    item.value = {
      name: payload.name,
      rarity,
      baseType: payload.base_type,
      itemLevel: payload.item_level,
      itemClass: payload.item_class,
      influence: payload.influence,
      corrupted: payload.corrupted,
      corruptedFilter: payload.corrupted,
      baseStats: payload.base_stats.map(s => ({
        id: s.id, label: s.label, value: s.value,
        min: (s.id === 'gem_level' || s.id === 'sockets') ? s.value : Math.floor(s.value * 0.8),
        max: null,
        enabled: filtersOn || s.id === 'gem_level' || s.id === 'sockets',
      })),
      typeEnabled: rarity === 'currency' || rarity === 'gem',
      ilvlEnabled: false,
      ilvlMin: payload.item_level,
      affixes: [
        ...computePseudoStats(payload.mods, filtersOn),
        ...payload.mods.map((mod, i) => ({
          id: String(i),
          text: mod.text, value: mod.value,
          min: mod.value >= 5 ? Math.floor(mod.value * 0.8) : null,
          max: null,
          enabled: filtersOn,
          stat_id: mod.stat_id ?? null,
          mod_group: mod.mod_group,
        })),
      ],
    }
    results.value = []

    if (autoSearch) nextTick(() => runSearch())
  }))
})

onUnmounted(() => {
  document.removeEventListener('mousemove', onDragMove)
  document.removeEventListener('mouseup', stopDrag)
  document.removeEventListener('pointerlockchange', onPointerLockChange)
  unlisteners.forEach(fn => fn())
})

async function openTradeSite() {
  handleInteract()
  if (lastTradeUrl.value) {
    await openUrl(lastTradeUrl.value)
    return
  }
  // Fallback: open the league search page with no query
  const league = encodeURIComponent(selectedLeague.value || 'Standard')
  const url = gameMode.value === 'poe1'
    ? `https://www.pathofexile.com/trade/search/${league}`
    : `https://www.pathofexile.com/trade2/search/${league}`
  await openUrl(url)
}
</script>

<template>
  <div class="overlay" @mousedown="handleInteract">

    <!-- ── Header ──────────────────────────────────────── -->
    <div class="header" @mousedown="startDrag">
      <div class="game-toggle">
        <button
          :class="['toggle-btn', { active: gameMode === 'poe1' }]"
          @click.stop="setGameMode('poe1')"
        >PoE 1</button>
        <button
          :class="['toggle-btn', { active: gameMode === 'poe2' }]"
          @click.stop="setGameMode('poe2')"
        >PoE 2</button>
      </div>
      <div class="header-right">
        <button :class="['gear-btn', { active: showSettings }]" @click.stop="toggleSettings">
          ⚙<span :class="['gear-dot', sessionIsSet ? 'gear-dot-ok' : 'gear-dot-warn']" />
        </button>
        <button class="close-btn" @click.stop="$emit('close')">✕</button>
      </div>
    </div>

    <!-- ── Settings panel ───────────────────────────────── -->
    <template v-if="showSettings">
      <div class="settings-panel">
        <div class="section">
          <div class="section-title">SESSION COOKIE</div>

          <div :class="['session-status', sessionIsSet ? 'status-ok' : 'status-warn']">
            {{ sessionIsSet ? 'Session configured' : 'No session — API calls will fail' }}
          </div>

          <button
            class="action-btn primary detect-btn"
            :disabled="detectStatus === 'busy'"
            @click.stop="autoDetect"
          >{{ detectStatus === 'busy' ? 'Detecting...' : 'Detect from Firefox' }}</button>

          <div v-if="detectStatus === 'ok'"  class="detect-msg detect-ok">Detected and saved</div>
          <div v-if="detectStatus === 'err'" class="detect-msg detect-err">{{ detectError }}</div>

          <div class="settings-or">— or enter manually —</div>

          <input
            type="password"
            class="session-input"
            v-model="sessionInput"
            :placeholder="sessionIsSet ? '••••••••  (leave blank to keep current)' : 'Paste POESESSID value here'"
            autocomplete="off"
            spellcheck="false"
            @click.stop
          />
          <p class="settings-hint">
            Log in at pathofexile.com → F12 → Application → Cookies → POESESSID
          </p>

          <div class="settings-actions">
            <button class="action-btn secondary" @click.stop="showSettings = false">Cancel</button>
            <button class="action-btn primary"   @click.stop="saveSession">Save</button>
          </div>
        </div>
      </div>
    </template>

    <!-- ── Main content ──────────────────────────────────── -->
    <template v-else>

    <!-- ── Item Info ───────────────────────────────────── -->
    <div class="item-info">
      <!-- Left: name / type / level -->
      <div class="item-info-left">
        <template v-if="searching">
          <div class="searching-label">Searching<span class="searching-dots"></span></div>
          <div class="item-base searching-sub">reading clipboard...</div>
        </template>
        <template v-else>
          <div :class="['item-name', rarityClass]">{{ item.name }}</div>
          <label class="item-filter-row" @click.stop="item.typeEnabled = !item.typeEnabled; handleInteract()">
            <input class="item-filter-cb" type="checkbox" v-model="item.typeEnabled" @click.stop />
            <span :class="['item-base', { 'filter-active': item.typeEnabled }]">{{ item.baseType }}</span>
          </label>
          <div class="item-meta">
            <label v-if="item.rarity !== 'gem'" class="item-filter-row" @click.stop="item.ilvlEnabled = !item.ilvlEnabled; handleInteract()">
              <input class="item-filter-cb" type="checkbox" v-model="item.ilvlEnabled" @click.stop />
              <span :class="{ 'filter-active': item.ilvlEnabled }">Item Level {{ item.itemLevel }}</span>
            </label>
            <span v-if="item.influence" class="influence">· {{ item.influence }}</span>
            <label v-if="item.corrupted" class="item-filter-row" @click.stop="item.corruptedFilter = !item.corruptedFilter; handleInteract()">
              <input class="item-filter-cb" type="checkbox" v-model="item.corruptedFilter" @click.stop />
              <span :class="['corrupted-tag', { 'filter-active': item.corruptedFilter }]">Corrupted</span>
            </label>
          </div>
        </template>
      </div>
      <!-- Right: league / currency -->
      <div class="item-info-right">
        <div class="info-select-row">
          <span class="info-select-label">League</span>
          <select
            class="info-select"
            :value="selectedLeague"
            :disabled="loadingLeagues"
            :class="{ 'select-loading': loadingLeagues }"
            @change="onLeagueChange"
          >
            <option v-if="loadingLeagues" value="" disabled>…</option>
            <option v-for="l in leagues" :key="l.id" :value="l.id">{{ l.text }}</option>
          </select>
        </div>
        <div class="info-select-row">
          <span class="info-select-label">Currency</span>
          <select class="info-select" v-model="selectedCurrency" @change="onCurrencyChange">
            <option v-for="c in currencies" :key="c.id" :value="c.id">{{ c.label }}</option>
          </select>
        </div>
      </div>
    </div>

    <template v-if="item.baseStats.length">
      <div class="sep" />

      <!-- ── Item Filters ───────────────────────────────── -->
      <div class="section">
        <div class="section-title">ITEM FILTERS</div>

        <label
          v-for="stat in item.baseStats"
          :key="stat.id"
          class="affix-row"
          @click="handleInteract"
        >
          <input class="affix-checkbox" type="checkbox" v-model="stat.enabled" />
          <span :class="['affix-text', { dimmed: !stat.enabled }]">{{ stat.label }}</span>
          <div class="affix-range">
            <input
              class="range-input"
              type="number"
              placeholder="min"
              v-model.number="stat.min"
              :disabled="!stat.enabled"
              @focus="onRangeInputFocus"
              @blur="onRangeInputBlur"
              @click.stop
            />
            <input
              class="range-input"
              type="number"
              placeholder="max"
              v-model.number="stat.max"
              :disabled="!stat.enabled"
              @focus="onRangeInputFocus"
              @blur="onRangeInputBlur"
              @click.stop
            />
          </div>
        </label>
      </div>
    </template>

    <div v-if="item.affixes.length" class="sep" />

    <!-- ── Affixes ─────────────────────────────────────── -->
    <div v-if="item.affixes.length" class="section">

      <!-- Pseudo stats sub-group -->
      <template v-if="pseudoAffixes.length">
        <div class="section-title">PSEUDO STATS</div>
        <div class="affix-list" :class="{ 'affix-loading': searching }">
          <label
            v-for="affix in pseudoAffixes"
            v-show="!searching"
            :key="affix.id"
            class="affix-row"
            @click="handleInteract"
          >
            <input class="affix-checkbox" type="checkbox" v-model="affix.enabled" />
            <span :class="['affix-text', 'pseudo-stat-text', { dimmed: !affix.enabled }]">
              {{ affix.text }}
            </span>
            <span class="pseudo-stat-value">{{ affix.value }}</span>
            <div class="affix-range">
              <input class="range-input" type="number" placeholder="min"
                v-model.number="affix.min" :disabled="!affix.enabled"
                @focus="onRangeInputFocus" @blur="onRangeInputBlur" @click.stop />
              <input class="range-input" type="number" placeholder="max"
                v-model.number="affix.max" :disabled="!affix.enabled"
                @focus="onRangeInputFocus" @blur="onRangeInputBlur" @click.stop />
            </div>
          </label>
        </div>
        <div v-if="corruptedAffixes.length || implicitAffixes.length || explicitAffixes.length" class="subsep" />
      </template>

      <!-- Corruption implicits sub-group -->
      <template v-if="corruptedAffixes.length">
        <div class="section-title">CORRUPTION IMPLICITS</div>
        <div class="affix-list" :class="{ 'affix-loading': searching }">
          <label
            v-for="affix in corruptedAffixes"
            v-show="!searching"
            :key="affix.id"
            class="affix-row"
            @click="handleInteract"
          >
            <input class="affix-checkbox" type="checkbox" v-model="affix.enabled" />
            <span :class="['affix-text', { dimmed: !affix.enabled }]">
              {{ formatAffix(affix.text, affix.value) }}
            </span>
            <div class="affix-range">
              <input class="range-input" type="number" placeholder="min"
                v-model.number="affix.min" :disabled="!affix.enabled"
                @focus="onRangeInputFocus" @blur="onRangeInputBlur" @click.stop />
              <input class="range-input" type="number" placeholder="max"
                v-model.number="affix.max" :disabled="!affix.enabled"
                @focus="onRangeInputFocus" @blur="onRangeInputBlur" @click.stop />
            </div>
          </label>
        </div>
        <div v-if="implicitAffixes.length || explicitAffixes.length" class="subsep" />
      </template>

      <!-- Implicit sub-group -->
      <template v-if="implicitAffixes.length">
        <div class="section-title">IMPLICITS</div>
        <div class="affix-list" :class="{ 'affix-loading': searching }">
          <label
            v-for="affix in implicitAffixes"
            v-show="!searching"
            :key="affix.id"
            class="affix-row"
            @click="handleInteract"
          >
            <input class="affix-checkbox" type="checkbox" v-model="affix.enabled" />
            <span :class="['affix-text', { dimmed: !affix.enabled }]">
              {{ formatAffix(affix.text, affix.value) }}
            </span>
            <div class="affix-range">
              <input class="range-input" type="number" placeholder="min"
                v-model.number="affix.min" :disabled="!affix.enabled"
                @focus="onRangeInputFocus" @blur="onRangeInputBlur" @click.stop />
              <input class="range-input" type="number" placeholder="max"
                v-model.number="affix.max" :disabled="!affix.enabled"
                @focus="onRangeInputFocus" @blur="onRangeInputBlur" @click.stop />
            </div>
          </label>
        </div>
        <div v-if="explicitAffixes.length" class="subsep" />
      </template>

      <!-- Explicit affixes -->
      <div class="section-title">AFFIXES</div>
      <div class="affix-list" :class="{ 'affix-loading': searching }">
        <label
          v-for="affix in explicitAffixes"
          v-show="!searching"
          :key="affix.id"
          class="affix-row"
          @click="handleInteract"
        >
          <input class="affix-checkbox" type="checkbox" v-model="affix.enabled" />
          <span :class="['affix-text', { dimmed: !affix.enabled }]">
            {{ formatAffix(affix.text, affix.value) }}
          </span>
          <div class="affix-range">
            <input class="range-input" type="number" placeholder="min"
              v-model.number="affix.min" :disabled="!affix.enabled"
              @focus="onRangeInputFocus" @blur="onRangeInputBlur" @click.stop />
            <input class="range-input" type="number" placeholder="max"
              v-model.number="affix.max" :disabled="!affix.enabled"
              @focus="onRangeInputFocus" @blur="onRangeInputBlur" @click.stop />
          </div>
        </label>
      </div>
    </div>

    <div class="sep" />

    <!-- ── Search Options ─────────────────────────────── -->
    <div class="section options-section">
      <div class="buyout-group">
        <button :class="['opt-btn', { active: buyoutType === 'iob' }]"      @click="buyoutType = 'iob'">Instant Buyout</button>
        <button :class="['opt-btn', { active: buyoutType === 'inperson' }]" @click="buyoutType = 'inperson'">In Person</button>
        <button :class="['opt-btn', { active: buyoutType === 'both' }]"     @click="buyoutType = 'both'">Both</button>
      </div>
    </div>

    <div class="sep" />

    <!-- ── Actions ────────────────────────────────────── -->
    <div class="actions">
      <button class="action-btn primary" @click="runSearch" :disabled="searching">Search</button>
      <button class="action-btn secondary" @click="openTradeSite">Open Trade Site ↗</button>
    </div>

    <div class="sep" />

    <!-- ── Results ────────────────────────────────────── -->
    <div class="section results-section">
      <div class="results-header">
        <span class="section-title" style="margin-bottom:0">RESULTS</span>
        <span v-if="medianPrice !== null" class="median">
          median <span class="median-price">{{ medianPrice }}</span>
          <span class="median-curr"> {{ medianCurrency }}</span>
        </span>
      </div>
      <div v-if="searchError" class="search-error">{{ searchError }}</div>
      <div v-else-if="searching" class="results-loading">Searching…</div>
      <div v-else class="results-list">
        <div v-if="!results.length && medianPrice === null" class="results-empty">No results</div>
        <div v-for="(r, i) in results" :key="i" class="result-row">
          <span class="result-price">
            {{ r.price }} <span class="result-curr">{{ r.currency.charAt(0).toUpperCase() + r.currency.slice(1) }}</span>
          </span>
          <span class="result-seller">{{ r.seller }}</span>
          <span :class="['result-dot', `dot-${r.status}`]">●</span>
        </div>
      </div>
    </div>

    </template><!-- end v-else main content -->
  </div>
</template>

<style scoped>
/* ── Overlay shell ──────────────────────────────────── */
.overlay {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg-primary);
  border: 1px solid var(--border-accent);
  border-radius: 6px;
  box-shadow: var(--shadow);
  overflow: hidden;
}

/* ── Header ─────────────────────────────────────────── */
.header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--border);
  cursor: grab;
  flex-shrink: 0;
}
.header:active { cursor: grabbing; }

.game-toggle {
  display: flex;
  gap: 4px;
}
.toggle-btn {
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  background: transparent;
  border: 1px solid var(--border);
  border-radius: 3px;
  color: var(--text-muted);
  cursor: pointer;
  transition: all 0.15s;
}
.toggle-btn:hover  { border-color: var(--accent-dim); color: var(--text); }
.toggle-btn.active { border-color: var(--accent); color: var(--accent); background: rgba(200,168,75,0.08); }

.header-right {
  display: flex;
  align-items: center;
  gap: 5px;
}

.gear-btn {
  width: 22px;
  height: 22px;
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  background: transparent;
  border: 1px solid var(--border);
  border-radius: 3px;
  color: var(--text-muted);
  cursor: pointer;
  transition: all 0.15s;
}
.gear-btn:hover  { border-color: var(--accent-dim); color: var(--text); }
.gear-btn.active { border-color: var(--accent-dim); color: var(--accent); background: rgba(200,168,75,0.08); }

.gear-dot {
  position: absolute;
  top: 2px;
  right: 2px;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  pointer-events: none;
}
.gear-dot-ok   { background: var(--online); }
.gear-dot-warn { background: var(--afk); }

.close-btn {
  width: 22px;
  height: 22px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  background: transparent;
  border: 1px solid var(--border);
  border-radius: 3px;
  color: var(--text-muted);
  cursor: pointer;
  transition: all 0.15s;
}
.close-btn:hover { border-color: #8b2020; color: #cc4444; background: rgba(139,32,32,0.15); }

/* ── Settings panel ──────────────────────────────────── */
.settings-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.settings-hint {
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.55;
  margin-bottom: 10px;
}
.settings-hint strong { color: var(--text-label); }
.session-input {
  width: 100%;
  padding: 6px 8px;
  font-size: 11px;
  font-family: monospace;
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: 3px;
  color: var(--text);
  outline: none;
  margin-bottom: 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.session-input:focus { border-color: var(--accent-dim); }
.settings-actions { display: flex; gap: 8px; }

.session-status {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  padding: 4px 0 10px;
}
.status-ok   { color: var(--online); }
.status-warn { color: var(--afk); }

.detect-btn { width: 100%; margin-bottom: 6px; }

.detect-msg {
  font-size: 11px;
  margin-bottom: 8px;
}
.detect-ok  { color: var(--online); }
.detect-err { color: #cc4444; line-height: 1.4; }

.settings-or {
  font-size: 10px;
  color: var(--text-muted);
  text-align: center;
  letter-spacing: 0.06em;
  margin: 8px 0;
}

/* ── Item Info ───────────────────────────────────────── */
.item-info {
  padding: 12px 16px 10px;
  flex-shrink: 0;
  display: flex;
  gap: 10px;
  align-items: flex-start;
}
.item-info-left {
  flex: 1;
  min-width: 0; /* allows text-overflow to work */
}
.item-name {
  font-size: 17px;
  font-weight: 700;
  letter-spacing: 0.03em;
  line-height: 1.2;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.rarity-rare      { color: var(--rare); text-shadow: 0 0 12px rgba(255,215,0,0.3); }
.rarity-magic     { color: var(--magic); }
.rarity-unique    { color: var(--unique); }
.rarity-normal    { color: var(--normal); }
.rarity-currency  { color: #c8a96e; }
.rarity-gem       { color: #1ba29b; }
.rarity-divination{ color: #9c6eca; }

.item-filter-row {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  cursor: pointer;
  user-select: none;
}
.item-filter-cb {
  width: 10px;
  height: 10px;
  flex-shrink: 0;
  margin: 0;
  cursor: pointer;
  accent-color: var(--accent);
}
.filter-active { color: var(--accent) !important; }

.item-base {
  font-size: 12px;
  color: var(--text-muted);
  margin-top: 2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.item-meta {
  font-size: 11px;
  color: var(--text-muted);
  margin-top: 4px;
  display: flex;
  align-items: center;
  gap: 6px;
}
.influence { color: var(--currency); }
.corrupted-tag { color: #b74545; }
.corrupted-tag.filter-active { color: #ff6b6b !important; }
.subsep { height: 1px; background: var(--border); margin: 3px 0 4px; opacity: 0.4; }

.item-info-right {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
  align-items: flex-end;
}
.info-select-row {
  display: flex;
  align-items: center;
  gap: 5px;
}
.info-select-label {
  font-size: 10px;
  color: var(--text-label);
  white-space: nowrap;
}
.info-select {
  background: var(--bg-input);
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='8' height='5'%3E%3Cpath d='M0 0l4 5 4-5z' fill='%236a6658'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 6px center;
  border: 1px solid var(--border);
  color: var(--text-muted);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.03em;
  padding: 3px 22px 3px 5px;
  border-radius: 3px;
  max-width: 110px;
  cursor: pointer;
  transition: all 0.15s;
  -webkit-appearance: none;
  appearance: none;
}
.info-select:hover { border-color: var(--accent-dim); color: var(--text); }
.info-select:focus { outline: none; border-color: var(--accent-dim); color: var(--text); }
.info-select option {
  background: var(--bg-input);
  color: var(--text);
}

/* ── Separator ───────────────────────────────────────── */
.sep {
  height: 1px;
  background: var(--border);
  flex-shrink: 0;
}

/* ── Section ─────────────────────────────────────────── */
.section {
  padding: 6px 12px;
  flex-shrink: 0;
}
.section-title {
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.1em;
  color: var(--text-label);
  margin-bottom: 4px;
}

/* ── Affixes ─────────────────────────────────────────── */
.affix-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
  max-height: 260px;
  overflow-y: auto;
  overflow-x: hidden;
}
.affix-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 4px;
  border-radius: 3px;
  cursor: pointer;
  transition: background 0.1s;
}
.affix-row:hover { background: var(--bg-hover); }

.affix-checkbox {
  width: 13px;
  height: 13px;
  flex-shrink: 0;
  accent-color: var(--accent);
  cursor: pointer;
}
.affix-text {
  flex: 1;
  font-size: 12px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: color 0.15s;
}
.affix-text.dimmed { color: var(--text-muted); }

.affix-range {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}
.range-input {
  width: 48px;
  padding: 2px 5px;
  font-size: 11px;
  text-align: center;
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: 3px;
  color: var(--text);
  outline: none;
  transition: border-color 0.15s;
  -moz-appearance: textfield;
}
.range-input::-webkit-inner-spin-button,
.range-input::-webkit-outer-spin-button { -webkit-appearance: none; }
.range-input:focus   { border-color: var(--accent-dim); }
.range-input:disabled { color: var(--text-muted); cursor: not-allowed; }

/* ── Options ─────────────────────────────────────────── */
.options-section { display: flex; flex-direction: column; gap: 8px; }

.buyout-group { display: flex; gap: 4px; }
.opt-btn {
  flex: 1;
  padding: 5px 6px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.03em;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: 3px;
  color: var(--text-muted);
  cursor: pointer;
  transition: all 0.15s;
}
.opt-btn:hover  { border-color: var(--accent-dim); color: var(--text); }
.opt-btn.active { border-color: var(--accent); color: var(--accent); background: rgba(200,168,75,0.1); }

.select-loading { opacity: 0.5; cursor: not-allowed; }

/* ── Actions ─────────────────────────────────────────── */
.actions {
  display: flex;
  gap: 8px;
  padding: 6px 12px;
  flex-shrink: 0;
}
.action-btn {
  flex: 1;
  padding: 7px 12px;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.04em;
  border-radius: 3px;
  cursor: pointer;
  transition: all 0.15s;
  border: 1px solid;
}
.action-btn.primary {
  background: rgba(200,168,75,0.15);
  border-color: var(--accent-dim);
  color: var(--accent);
}
.action-btn.primary:hover {
  background: rgba(200,168,75,0.25);
  border-color: var(--accent);
}
.action-btn.secondary {
  background: var(--bg-input);
  border-color: var(--border);
  color: var(--text-muted);
}
.action-btn.secondary:hover {
  border-color: var(--accent-dim);
  color: var(--text);
}

/* ── Results ─────────────────────────────────────────── */
.results-section {
  flex: 1;
  min-height: 0; /* required for flex children to scroll */
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.results-header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  margin-bottom: 6px;
}
.median { font-size: 11px; color: var(--text-muted); }
.median-price { color: var(--currency); font-weight: 700; }
.median-curr  { font-size: 10px; }

.results-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.result-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 5px 6px;
  border-radius: 3px;
  font-size: 12px;
  transition: background 0.1s;
}
.result-row:hover { background: var(--bg-hover); }

.result-price { font-weight: 700; color: var(--currency); white-space: nowrap; min-width: 40px; }
.result-curr  { font-weight: 400; font-size: 10px; color: var(--text-muted); }
.result-seller { flex: 1; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 11px; }
.result-dot { font-size: 9px; flex-shrink: 0; }
.dot-online  { color: var(--online); }
.dot-afk     { color: var(--afk); }
.dot-offline { color: var(--offline); }

/* ── Loading state ───────────────────────────────────── */
.searching-label {
  font-size: 17px;
  font-weight: 700;
  color: var(--text-label);
  letter-spacing: 0.03em;
}
.searching-sub {
  color: var(--text-muted);
  font-style: italic;
}
.searching-dots::after {
  content: '';
  animation: dots 1.4s steps(4, end) infinite;
}
@keyframes dots {
  0%   { content: ''; }
  25%  { content: '.'; }
  50%  { content: '..'; }
  75%  { content: '...'; }
  100% { content: ''; }
}
.affix-loading {
  min-height: 32px;
  opacity: 0.3;
}

/* ── Pseudo stats ────────────────────────────────────── */
.pseudo-stat-text {
  color: #7eaec8 !important;
}
.pseudo-stat-text.dimmed {
  color: var(--text-muted) !important;
}
.pseudo-stat-value {
  font-size: 11px;
  color: #7eaec8;
  font-weight: 600;
  flex-shrink: 0;
  min-width: 28px;
  text-align: right;
  opacity: 0.8;
}
</style>
