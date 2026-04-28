<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  export let activeTab: 'viz' | 'anlys' = 'viz';
  const dispatch = createEventDispatcher<{ tabChange: 'viz' | 'anlys' }>();

  function selTab(tab: 'viz' | 'anlys') {
    activeTab = tab;
    dispatch('tabChange', tab);
  }
</script>

<div class="tab-container">
  <div class="tabs">

    <button
      class="tab"
      class:active={activeTab === 'viz'}
      on:click={() => selTab('viz')}
    >
      <svg class="tab-icon" width="16" height="16" viewBox="0 0 16 16" fill="none">
        <circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="2" />
        <path d="M8 2 L8 8 L12 5" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
      </svg>
      Chromosomal Flow Chart
    </button>

    <button
      class="tab"
      class:active={activeTab === 'anlys'}
      on:click={() => selTab('anlys')}
    >
      <svg class="tab-icon" width="16" height="16" viewBox="0 0 16 16" fill="none">
        <rect x="2" y="10" width="3" height="4" fill="currentColor" />
        <rect x="6.5" y="6" width="3" height="8" fill="currentColor" />
        <rect x="11" y="3" width="3" height="11" fill="currentColor" />
      </svg>
      Analytic Browser
    </button>
  </div>
</div>

<style>
  .tab-container {
    margin-bottom: 2rem;
    border-bottom: 2px solid var(--border-color);
  }

  .tabs {
    display: flex;
    gap: 0.5rem;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1.5rem;
    background: transparent;
    border: none;
    border-bottom: 3px solid transparent;
    color: var(--text-secondary);
    font-weight: 500;
    font-size: 0.875rem;
    cursor: pointer;
    transition: all 0.2s;
    position: relative;
    top: 2px;
  }

  .tab:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .tab.active {
    color: var(--accent-primary);
    border-bottom-color: var(--accent-primary);
    background: transparent;
  }

  .tab-icon {
    flex-shrink: 0;
  }
</style>