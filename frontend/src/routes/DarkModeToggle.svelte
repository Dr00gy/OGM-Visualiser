<script lang="ts">
import { darkMode } from '$lib/darkModeStore';
</script>

<button class="dark-mode-toggle" on:click={() => darkMode.toggle()} aria-label="Toggle dark mode">
  {#if $darkMode}
    <!-- TODO: SVG replacement.". -->
    <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
      <circle cx="10" cy="10" r="4" fill="currentColor"/>
      <path d="M10 2v2M10 16v2M18 10h-2M4 10H2M15.657 4.343l-1.414 1.414M5.757 14.243l-1.414 1.414M15.657 15.657l-1.414-1.414M5.757 5.757L4.343 4.343" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
    </svg>
  {:else}
    <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
      <path d="M17.293 13.293A8 8 0 016.707 2.707a8.001 8.001 0 1010.586 10.586z" fill="currentColor"/>
    </svg>
  {/if}
</button>

<style>
  .dark-mode-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 2.5rem;
    height: 2.5rem;
    background: var(--bg-secondary);
    border: 2px solid var(--border-color);
    border-radius: 0.5rem;
    color: var(--text-primary);
    cursor: pointer;
    transition: all 0.2s;
  }
  .dark-mode-toggle:hover {
    background: var(--bg-hover);
    border-color: var(--accent-primary);
    color: var(--accent-primary);
    transform: scale(1.05);
  }

  .dark-mode-toggle svg {
    flex-shrink: 0;
  }
</style>