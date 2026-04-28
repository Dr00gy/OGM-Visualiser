<script lang="ts">
  import { onDestroy } from 'svelte';

  export let options: number[] = [];
  export let value: string = '';
  export let id: string = 'seq-picker';
  export let placeholder: string = 'Type or pick a sequence ID…';

  let inText: string = value;
  let dropOpen: boolean = false;
  let rootEl: HTMLDivElement;
  let scrollEl: HTMLDivElement;
  let scrollTop: number = 0;

  const ITEM_H = 28;
  const SCROLL_H = 280;
  const WIN_SIZE = Math.ceil(SCROLL_H / ITEM_H) * 3;

  // Sync input when `value` changes externally (and the input isn't focused).
  $: if (typeof document !== 'undefined' &&
         value !== inText &&
         document.activeElement !== inputRef) {
    inText = value;
  }

  let inputRef: HTMLInputElement | null = null;

  $: filtered = (() => {
    const q = inText.trim();
    if (q === '') return options;
    const out: number[] = [];
    for (const sid of options) {
      if (sid.toString().includes(q)) out.push(sid);
    }
    return out;
  })();

  $: suggestions = (() => {
    const q = inText.trim();
    if (q === '' || filtered.length === 0) return [] as number[];

    const prefix: number[] = [];
    for (const sid of filtered) {
      if (sid.toString().startsWith(q)) {
        prefix.push(sid);
        if (prefix.length >= 5) break;
      }
    }
    if (prefix.length === 0) return filtered.slice(0, 5);
    return prefix;
  })();

  // Virtualization windowing
  $: winStart = (() => {
    const vpFirst = Math.floor(scrollTop / ITEM_H);
    const pad = Math.floor((WIN_SIZE - SCROLL_H / ITEM_H) / 2);
    return Math.max(0, vpFirst - pad);
  })();

  $: winItems = filtered.slice(winStart, winStart + WIN_SIZE);
  $: totH = filtered.length * ITEM_H;
  $: winOff = winStart * ITEM_H;

  function onInFocus() {
    dropOpen = true;
  }

  function onInChange() {
    scrollTop = 0;
    if (scrollEl) scrollEl.scrollTop = 0;
  }

  function commit(sid: number) {
    inText = sid.toString();
    value = inText;
    dropOpen = false;
  }

  function onInBlur(e: FocusEvent) {
    const next = e.relatedTarget as HTMLElement | null;
    if (rootEl && next && rootEl.contains(next)) return;

    const typed = inText.trim();
    if (typed === '') {
      value = '';
    } else {
      const n = parseInt(typed, 10);
      if (!Number.isNaN(n) && n >= 0) {
        value = n.toString();
      } else {
        inText = value;
      }
    }
    dropOpen = false;
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      dropOpen = false;
      inputRef?.blur();
    } else if (e.key === 'Enter') {
      if (suggestions.length >= 1) {
        commit(suggestions[0]);
        inputRef?.blur();
      } else {
        onInBlur(new FocusEvent('blur'));
        inputRef?.blur();
      }
    }
  }

  function onClear() {
    inText = '';
    value = '';
    scrollTop = 0;
    if (scrollEl) scrollEl.scrollTop = 0;
    inputRef?.focus();
  }

  function onScroll() {
    if (!scrollEl) return;
    scrollTop = scrollEl.scrollTop;
  }

  function onDocClick(e: MouseEvent) {
    if (rootEl && !rootEl.contains(e.target as Node)) {
      dropOpen = false;
    }
  }

  $: if (typeof document !== 'undefined') {
    if (dropOpen) {
      document.addEventListener('click', onDocClick, { capture: true });
    } else {
      document.removeEventListener('click', onDocClick, { capture: true });
    }
  }

  onDestroy(() => {
    if (typeof document !== 'undefined') {
      document.removeEventListener('click', onDocClick, { capture: true });
    }
  });
</script>

<div class="picker-root" bind:this={rootEl}>
  <div class="picker-input-row">
    <input
      bind:this={inputRef}
      {id}
      type="text"
      inputmode="numeric"
      class="picker-input"
      {placeholder}
      bind:value={inText}
      on:focus={onInFocus}
      on:input={onInChange}
      on:blur={onInBlur}
      on:keydown={onKeyDown}
      autocomplete="off"
    />
    {#if value !== ''}
      <button
        type="button"
        class="picker-clear"
        on:click={onClear}
        title="Clear"
        tabindex="-1"
      >
        ✕
      </button>
    {/if}
  </div>

  {#if dropOpen && options.length > 0}
    <div class="picker-dropdown" role="listbox">
      {#if suggestions.length > 0}
        <div class="picker-suggestions">
          {#each suggestions as sid (sid)}
            <button
              type="button"
              class="picker-suggestion"
              class:active={value === sid.toString()}
              on:click={() => commit(sid)}
            >
              {sid}
            </button>
          {/each}
        </div>
      {/if}

      <div
        class="picker-scroll"
        bind:this={scrollEl}
        on:scroll={onScroll}
      >
        <div class="picker-spacer" style="height: {totH}px;">
          <div
            class="picker-window"
            style="transform: translateY({winOff}px);"
          >
            {#each winItems as sid (sid)}
              <button
                type="button"
                class="picker-item"
                class:active={value === sid.toString()}
                on:click={() => commit(sid)}
                style="height: {ITEM_H}px;"
              >
                {sid}
              </button>
            {/each}
          </div>
        </div>
      </div>

      <div class="picker-footer">
        {filtered.length.toLocaleString()} of {options.length.toLocaleString()}
        {inText ? 'matching' : 'total'}
      </div>
    </div>
  {/if}
</div>

<style>
  .picker-root {
    position: relative;
    width: 100%;
  }

  .picker-input-row {
    display: flex;
    gap: 0.25rem;
    align-items: stretch;
  }

  .picker-input {
    flex: 1;
    padding: 0.5rem;
    border: 1px solid var(--border-color-dark);
    border-radius: 0.375rem;
    font-size: 0.8rem;
    background: var(--bg-primary);
    color: var(--text-primary);
    box-sizing: border-box;
  }

  .picker-input:focus {
    outline: 2px solid var(--accent-primary);
    outline-offset: -2px;
  }

  .picker-clear {
    padding: 0 0.5rem;
    background: transparent;
    color: var(--text-secondary);
    border: 1px solid var(--border-color);
    border-radius: 0.375rem;
    font-size: 0.7rem;
    cursor: pointer;
    flex-shrink: 0;
  }
  .picker-clear:hover {
    color: var(--text-primary);
    border-color: var(--border-color-dark);
  }

  .picker-dropdown {
    position: absolute;
    top: calc(100% + 0.25rem);
    left: 0;
    right: 0;
    background: var(--bg-primary);
    border: 1px solid var(--border-color-dark);
    border-radius: 0.375rem;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
    z-index: 100;
    overflow: hidden;
  }

  .picker-suggestions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    padding: 0.5rem;
    border-bottom: 1px solid var(--border-color);
    background: var(--bg-secondary);
  }

  .picker-suggestion {
    padding: 0.2rem 0.6rem;
    background: var(--bg-primary);
    color: var(--accent-primary);
    border: 1px solid var(--accent-primary);
    border-radius: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
  }
  .picker-suggestion:hover {
    background: var(--accent-light);
  }
  .picker-suggestion.active {
    background: var(--accent-primary);
    color: white;
  }

  .picker-scroll {
    max-height: 280px;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .picker-spacer {
    position: relative;
    width: 100%;
  }

  .picker-window {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    flex-direction: column;
  }

  .picker-item {
    text-align: left;
    padding: 0 0.75rem;
    background: transparent;
    color: var(--text-primary);
    border: none;
    border-bottom: 1px solid var(--border-color);
    font-size: 0.8rem;
    cursor: pointer;
    font-family: inherit;
    box-sizing: border-box;
    line-height: 1.2;
    display: flex;
    align-items: center;
  }
  .picker-item:hover {
    background: var(--bg-hover);
  }
  .picker-item.active {
    background: var(--accent-light);
    color: var(--accent-primary);
    font-weight: 600;
  }

  .picker-footer {
    padding: 0.375rem 0.75rem;
    border-top: 1px solid var(--border-color);
    background: var(--bg-secondary);
    font-size: 0.7rem;
    color: var(--text-tertiary);
    text-align: center;
  }
</style>