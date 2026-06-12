<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

defineEmits<{ done: [] }>()

const keybind = ref('KeyD')
const capturing = ref(false)
let captureHandler: ((e: KeyboardEvent) => void) | null = null

function displayMod()  { return 'Alt' }
function displayKey(code: string): string {
  if (code.startsWith('Key')) return code.slice(3)
  if (code === 'Backquote') return '`'
  return code
}

function stopCapture() {
  capturing.value = false
  invoke('set_overlay_keyboard', { enabled: false })
  if (captureHandler) {
    document.removeEventListener('keydown', captureHandler)
    captureHandler = null
  }
}

function startCapture() {
  capturing.value = true
  invoke('set_overlay_keyboard', { enabled: true })
  captureHandler = (e: KeyboardEvent) => {
    e.preventDefault()
    if (['Alt', 'Control', 'Shift', 'Meta'].includes(e.key)) return
    if (e.key === 'Escape') { stopCapture(); return }
    const code = e.code
    invoke<void>('set_keybind', { code })
      .then(() => { keybind.value = code })
      .catch(err => console.error('set_keybind:', err))
    stopCapture()
  }
  document.addEventListener('keydown', captureHandler)
}

onMounted(() => {
  invoke<string>('get_keybind').then(code => { keybind.value = code })
})

onUnmounted(() => { stopCapture() })
</script>

<template>
  <div class="setup-shell">

    <!-- ── Header bar ── -->
    <div class="topbar">
      <span class="topbar-title">First Time Setup</span>
      <button class="close-btn" @click="$emit('done')">✕</button>
    </div>

    <!-- ── Logo ── -->
    <div class="logo-wrap">
      <div class="logo-rule"><span class="logo-rule-gem">◆</span></div>
      <div class="logo-title">ExileWatch</div>
      <div class="logo-sub">Path of Exile Price Checker</div>
      <div class="logo-rule"><span class="logo-rule-gem">◆</span></div>
    </div>

    <!-- ── Divider ── -->
    <div class="panel-divider" />

    <!-- ── Keybind setup ── -->
    <div class="panel">
      <div class="panel-heading">⚙ Configure Your Hotkey</div>
      <p class="panel-desc">
        Hover any item in-game and press your hotkey to open the price overlay.
        Choose a key that doesn't conflict with your in-game binds.
      </p>

      <!-- Key display -->
      <div class="keybind-row">
        <div class="keycap-group">
          <div class="keycap mod">{{ displayMod() }}</div>
          <div class="keycap-plus">+</div>
          <div :class="['keycap', 'trigger', { pulse: capturing }]">
            {{ capturing ? '?' : displayKey(keybind) }}
          </div>
        </div>
        <button
          :class="['change-btn', { active: capturing }]"
          @click="capturing ? stopCapture() : startCapture()"
        >
          {{ capturing ? 'Esc to cancel' : 'Change' }}
        </button>
      </div>

      <p v-if="capturing" class="capture-status">
        ◆ Press any key…
      </p>

      <div class="warn-box">
        <span class="warn-icon">⚠</span>
        <span>
          If you use <strong>WASD movement</strong>, avoid those keys —
          pressing them will move your character and may close storage.
        </span>
      </div>
    </div>

    <!-- ── Divider ── -->
    <div class="panel-divider" />

    <!-- ── CTA ── -->
    <div class="cta-wrap">
      <button class="cta-btn" @click="$emit('done')">
        Enter the Exile
      </button>
      <p class="cta-hint">You can change this anytime from the ⚙ menu</p>
    </div>

  </div>
</template>

<style scoped>
.setup-shell {
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

/* ── Topbar ── */
.topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
.topbar-title {
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.12em;
  color: var(--text-label);
  text-transform: uppercase;
}
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

/* ── Logo ── */
.logo-wrap {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 36px 24px 28px;
  background: linear-gradient(180deg, rgba(200,168,75,0.07) 0%, transparent 100%);
  flex-shrink: 0;
}

.logo-rule {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 0;
}
.logo-rule::before,
.logo-rule::after {
  content: '';
  flex: 1;
  height: 1px;
  background: linear-gradient(90deg, transparent, var(--border-accent));
}
.logo-rule::after {
  background: linear-gradient(90deg, var(--border-accent), transparent);
}
.logo-rule-gem {
  font-size: 9px;
  color: var(--accent-dim);
  padding: 0 10px;
  flex-shrink: 0;
}

.logo-title {
  font-size: 36px;
  font-weight: 900;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--accent);
  text-shadow:
    0 0 20px rgba(200,168,75,0.6),
    0 0 60px rgba(200,168,75,0.2);
  margin: 14px 0 8px;
}

.logo-sub {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.22em;
  text-transform: uppercase;
  color: var(--text-muted);
  margin-bottom: 14px;
}

/* ── Divider ── */
.panel-divider {
  height: 1px;
  background: var(--border);
  flex-shrink: 0;
}

/* ── Panel ── */
.panel {
  padding: 22px 28px;
  flex-shrink: 0;
}

.panel-heading {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--text-label);
  margin-bottom: 10px;
}

.panel-desc {
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.65;
  margin-bottom: 20px;
}

/* ── Keybind display ── */
.keybind-row {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-bottom: 12px;
}

.keycap-group {
  display: flex;
  align-items: center;
  gap: 8px;
}

.keycap {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 6px 12px;
  background: var(--bg-secondary);
  border: 1px solid var(--border-accent);
  border-radius: 4px;
  font-family: monospace;
  font-weight: 800;
  letter-spacing: 0.06em;
  box-shadow: 0 3px 0 var(--accent-dim), 0 4px 6px rgba(0,0,0,0.5);
  user-select: none;
}
.keycap.mod {
  font-size: 13px;
  color: var(--text-muted);
  min-width: 46px;
}
.keycap.trigger {
  font-size: 18px;
  color: var(--accent);
  min-width: 46px;
  text-shadow: 0 0 12px rgba(200,168,75,0.4);
  transition: color 0.15s;
}
.keycap.pulse {
  color: var(--text-muted);
  animation: blink 0.9s steps(1) infinite;
  border-color: var(--accent-dim);
}
@keyframes blink {
  0%, 100% { opacity: 1; }
  50%       { opacity: 0.3; }
}

.keycap-plus {
  font-size: 16px;
  color: var(--border-accent);
  font-weight: 700;
  user-select: none;
}

.change-btn {
  padding: 7px 16px;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.05em;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: 3px;
  color: var(--text-muted);
  cursor: pointer;
  transition: all 0.15s;
  white-space: nowrap;
}
.change-btn:hover  { border-color: var(--accent-dim); color: var(--text); }
.change-btn.active {
  border-color: var(--accent);
  color: var(--accent);
  background: rgba(200,168,75,0.08);
}

.capture-status {
  font-size: 11px;
  color: var(--accent-dim);
  font-style: italic;
  letter-spacing: 0.06em;
  margin-bottom: 10px;
  animation: fadein 0.2s ease;
}
@keyframes fadein { from { opacity: 0 } to { opacity: 1 } }

.warn-box {
  display: flex;
  gap: 8px;
  padding: 10px 12px;
  background: rgba(192,120,48,0.08);
  border: 1px solid rgba(192,120,48,0.25);
  border-radius: 4px;
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.55;
  margin-top: 4px;
}
.warn-icon { color: var(--afk); flex-shrink: 0; }
.warn-box strong { color: var(--afk); font-weight: 700; }

/* ── CTA ── */
.cta-wrap {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 28px 28px 32px;
}

.cta-btn {
  width: 100%;
  padding: 14px;
  font-size: 14px;
  font-weight: 800;
  letter-spacing: 0.16em;
  text-transform: uppercase;
  background: linear-gradient(180deg, rgba(200,168,75,0.18) 0%, rgba(200,168,75,0.08) 100%);
  border: 1px solid var(--accent-dim);
  border-radius: 4px;
  color: var(--accent);
  cursor: pointer;
  transition: all 0.2s;
  box-shadow: inset 0 1px 0 rgba(200,168,75,0.15);
}
.cta-btn:hover {
  background: linear-gradient(180deg, rgba(200,168,75,0.3) 0%, rgba(200,168,75,0.15) 100%);
  border-color: var(--accent);
  box-shadow:
    inset 0 1px 0 rgba(200,168,75,0.2),
    0 0 20px rgba(200,168,75,0.15);
}
.cta-btn:active {
  transform: translateY(1px);
}

.cta-hint {
  font-size: 10px;
  color: var(--text-muted);
  letter-spacing: 0.06em;
  text-align: center;
}
</style>
