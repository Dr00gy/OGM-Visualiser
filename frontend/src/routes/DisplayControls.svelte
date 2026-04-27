<script lang="ts">
  export let showDups = false;
</script>

<div class="display-controls">
  <label class="toggle-label">
    <input type="checkbox" bind:checked={showDups} />
    <span class="toggle-slider"></span>
    Show self flow lines
  </label>
</div>

<style>
  .display-controls {
    margin-top: 1rem;
    gap: 2rem;
  }

  .toggle-label {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    cursor: pointer;
    font-weight: 400;
    color: var(--text-secondary);
    font-size: 0.875rem;
  }

  .toggle-label input {
    display: none;
  }

  .toggle-slider {
    width: 2.25rem;
    height: 1.125rem;
    background: var(--border-color-dark);
    border-radius: 1rem;
    position: relative;
    transition: background 0.2s;
    flex-shrink: 0;
  }
  .toggle-slider::before {
    content: '';
    position: absolute;
    width: 0.875rem;
    height: 0.875rem;
    background: var(--bg-primary);
    border-radius: 50%;
    top: 0.125rem;
    left: 0.125rem;
    transition: transform 0.2s;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }
  .toggle-label input:checked + .toggle-slider {
    background: var(--accent-primary);
  }

  .toggle-label input:checked + .toggle-slider::before {
    transform: translateX(1.125rem);
    background: var(--accent-light);
  }
</style>