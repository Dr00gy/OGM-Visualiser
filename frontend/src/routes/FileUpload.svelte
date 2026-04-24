<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  const dispatch = createEventDispatcher();
  const REFINEFINAL_NAME = 'exp_refineFinal1.xmap';
  const CONTIG_PATH = ['assembly', 'output', 'contigs', 'exp_refineFinal1', 'alignmol', 'merge'];

  interface GenomeZone {
    contigFiles: File[];
    refineFinalFile: File | null;
    dirName: string;
    dragging: boolean;
    dragCounter: number;
    missingRefineFinal: boolean;
  }

  let zones: GenomeZone[] = [
    { contigFiles: [], refineFinalFile: null, dirName: '', dragging: false, dragCounter: 0, missingRefineFinal: false },
    { contigFiles: [], refineFinalFile: null, dirName: '', dragging: false, dragCounter: 0, missingRefineFinal: false },
  ];
  let fileInputs: HTMLInputElement[] = [];

  function addZone() {
    if (zones.length < 3) {
      zones = [...zones, { contigFiles: [], refineFinalFile: null, dirName: '', dragging: false, dragCounter: 0, missingRefineFinal: false }];
    }
  }

  function removeZone(index: number) {
    zones = zones.filter((_, i) => i !== index);
  }

  async function walkDirectory(
    dir: FileSystemDirectoryEntry,
    segments: string[]
  ): Promise<FileSystemDirectoryEntry | null> {
    if (segments.length === 0) return dir;
    const [first, ...rest] = segments;
    return new Promise((resolve) => {
      dir.createReader().readEntries((entries) => {
        const match = entries.find(
          (e) => e.isDirectory && e.name === first
        ) as FileSystemDirectoryEntry | undefined;
        if (match) {
          walkDirectory(match, rest).then(resolve);
        } else {
          resolve(null);
        }
      });
    });
  }

  async function collectXmapsFromDir(dir: FileSystemDirectoryEntry): Promise<File[]> {
    const files: File[] = [];
    const reader = dir.createReader();
    while (true) {
      const entries: FileSystemEntry[] = await new Promise((resolve, reject) =>
        reader.readEntries(resolve, reject)
      );
      if (entries.length === 0) break;
      for (const entry of entries) {
        if (entry.isFile && entry.name.endsWith('.xmap')) {
          const file = await new Promise<File>((resolve, reject) =>
            (entry as FileSystemFileEntry).file(resolve, reject)
          );
          files.push(file);
        }
      }
    }
    return files;
  }

  async function collectFromDirectory(dir: FileSystemDirectoryEntry): Promise<{
    contigFiles: File[];
    refineFinalFile: File | null;
  }> {
    const contigFiles: File[] = [];
    let refineFinalFile: File | null = null;

    const contigDir = await walkDirectory(dir, CONTIG_PATH);
    if (contigDir) {
      const allFiles = await collectXmapsFromDir(contigDir);
      for (const f of allFiles) {
        if (f.name !== REFINEFINAL_NAME) {
          contigFiles.push(f);
        }
      }
    }

    const rootFiles = await collectXmapsFromDir(dir);
    refineFinalFile = rootFiles.find(f => f.name === REFINEFINAL_NAME) ?? null;

    return { contigFiles, refineFinalFile };
  }

  function handleChange(e: Event, zoneIndex: number) {
    const input = e.target as HTMLInputElement;
    const fileList = input.files;
    if (!fileList || fileList.length === 0) return;

    const allFiles = Array.from(fileList);
    const contigFiles: File[] = [];
    let refineFinalFile: File | null = null;
    let dirName = '';

    const contigPathStr = CONTIG_PATH.join('/');

    for (const f of allFiles) {
      const rel = (f as any).webkitRelativePath as string;
      if (!dirName) dirName = rel.split('/')[0];
      const normalized = rel.replace(/\\/g, '/');
      const parts = normalized.split('/');

      if (normalized.includes(contigPathStr) && f.name.endsWith('.xmap') && f.name !== REFINEFINAL_NAME) {
        contigFiles.push(f);
      } else if (parts.length === 2 && f.name === REFINEFINAL_NAME) {
        refineFinalFile = f;
      }
    }

    zones[zoneIndex].contigFiles = contigFiles;
    zones[zoneIndex].refineFinalFile = refineFinalFile;
    zones[zoneIndex].dirName = dirName;
    zones[zoneIndex].missingRefineFinal = refineFinalFile === null && contigFiles.length > 0;
    zones = [...zones];
    input.value = '';
  }

  async function handleDrop(e: DragEvent, zoneIndex: number) {
    e.preventDefault();
    e.stopPropagation();
    zones[zoneIndex].dragging = false;
    zones[zoneIndex].dragCounter = 0;
    zones = [...zones];

    const items = e.dataTransfer?.items;
    if (!items) return;

    for (let i = 0; i < items.length; i++) {
      const entry = items[i].webkitGetAsEntry?.();
      if (entry?.isDirectory) {
        const { contigFiles, refineFinalFile } = await collectFromDirectory(
          entry as FileSystemDirectoryEntry
        );
        if (contigFiles.length > 0 || refineFinalFile) {
          zones[zoneIndex].contigFiles = contigFiles;
          zones[zoneIndex].refineFinalFile = refineFinalFile;
          zones[zoneIndex].dirName = entry.name;
          zones[zoneIndex].missingRefineFinal = refineFinalFile === null && contigFiles.length > 0;
          zones = [...zones];
          break;
        }
      }
    }
  }

  function handleDragEnter(e: DragEvent, zoneIndex: number) {
    e.preventDefault();
    e.stopPropagation();
    zones[zoneIndex].dragCounter++;
    zones[zoneIndex].dragging = true;
    zones = [...zones];
  }

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
  }

  function handleDragLeave(e: DragEvent, zoneIndex: number) {
    e.preventDefault();
    e.stopPropagation();
    zones[zoneIndex].dragCounter--;
    if (zones[zoneIndex].dragCounter === 0) zones[zoneIndex].dragging = false;
    zones = [...zones];
  }

  function handleSubmit() {
    const populated = zones.filter(z => z.contigFiles.length > 0 && z.refineFinalFile !== null);
    if (populated.length < 2) return;
    dispatch('upload', populated.map(z => ({
      contigFiles: z.contigFiles,
      refineFinalFile: z.refineFinalFile!,
      dirName: z.dirName,
    })));
  }

  $: canSubmit = zones.filter(z => z.contigFiles.length > 0 && z.refineFinalFile !== null).length >= 2;
  $: zoneColors = ['#3b82f6', '#10b981', '#f59e0b'];
  $: hasAnyFiles = (z: GenomeZone) => z.contigFiles.length > 0 || z.refineFinalFile !== null;
</script>

<div class="uploader">
  <div class="zones">
    {#each zones as zone, i}
      <div
        class="zone"
        class:dragging={zone.dragging}
        class:filled={hasAnyFiles(zone)}
        style="--zone-color: {zoneColors[i]}"
        on:dragenter={(e) => handleDragEnter(e, i)}
        on:dragover={handleDragOver}
        on:dragleave={(e) => handleDragLeave(e, i)}
        on:drop={(e) => handleDrop(e, i)}
        role="region"
        aria-label="Genome {i + 1} upload zone"
      >
        <input
          bind:this={fileInputs[i]}
          type="file"
          webkitdirectory
          multiple
          on:change={(e) => handleChange(e, i)}
          style="display: none;"
        />
        <div class="zone-label">Genome {i + 1}</div>

        {#if hasAnyFiles(zone)}
          <div class="zone-filled">
            {#if zone.missingRefineFinal}
              <svg width="32" height="32" viewBox="0 0 32 32" fill="none">
                <circle cx="16" cy="16" r="14" fill="#f59e0b" opacity="0.15"/>
                <path d="M16 10v7M16 21v1.5" stroke="#f59e0b" stroke-width="2.5" stroke-linecap="round"/>
              </svg>
              <p class="dir-name">{zone.dirName}</p>
              <p class="file-count warn">{zone.contigFiles.length} contig file{zone.contigFiles.length !== 1 ? 's' : ''}</p>
              <p class="missing-warning">Missing exp_refineFinal1.xmap</p>
            {:else}
              <svg width="32" height="32" viewBox="0 0 32 32" fill="none">
                <circle cx="16" cy="16" r="14" fill="var(--zone-color)" opacity="0.15"/>
                <path d="M9 16l5 5 9-9" stroke="var(--zone-color)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
              <p class="dir-name">{zone.dirName}</p>
              <p class="file-count">{zone.contigFiles.length} contig file{zone.contigFiles.length !== 1 ? 's' : ''}</p>
              <p class="refinefinal-ok">✓ exp_refineFinal1.xmap</p>
            {/if}
            <button class="change-btn" on:click={() => fileInputs[i].click()}>Change folder</button>
          </div>
        {:else}
          <div class="zone-empty">
            <svg width="40" height="40" viewBox="0 0 40 40" fill="none">
              <path d="M20 12v16M12 20l8-8 8 8" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
              <path d="M12 30h16" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
            </svg>
            <p class="zone-hint">Drop genome folder here</p>
            <button class="pick-btn" on:click={() => fileInputs[i].click()}>Select Folder</button>
          </div>
        {/if}
        {#if i === 2}
          <button class="remove-zone" on:click={() => removeZone(i)} title="Remove this genome" aria-label="Remove genome {i + 1}">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M2 2l10 10M12 2L2 12" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
            </svg>
          </button>
        {/if}
      </div>
    {/each}
  </div>
  <div class="actions">
    {#if zones.length < 3}
      <button class="add-genome-btn" on:click={addZone}>
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
          <path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
        </svg>
        Add third genome
      </button>
    {:else}
      <div></div>
    {/if}
    <button class="submit-btn" disabled={!canSubmit} on:click={handleSubmit}>
      Run Matching
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
        <path d="M3 8h10M9 4l4 4-4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
    </button>
  </div>

  <p class="requirement">Select 2–3 genome folders.</p>
</div>

<style>
  .uploader { display: flex; flex-direction: column; gap: 1.25rem; }

  .zones { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 1rem; }

  .zone {
    position: relative;
    border: 2.5px dashed var(--border-color-dark);
    border-radius: 0.75rem;
    background: var(--bg-secondary);
    min-height: 220px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.25s ease;
    overflow: hidden;
  }

  .zone:hover { border-color: var(--zone-color); background: var(--bg-accent); }
  .zone.dragging {
    border-color: var(--zone-color);
    border-style: solid;
    background: var(--bg-accent);
    transform: scale(1.02);
  }
  .zone.filled { border-color: var(--zone-color); border-style: solid; }
  .zone-label {
    position: absolute;
    top: 0.625rem;
    left: 0.75rem;
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--zone-color);
    opacity: 0.8;
  }
  .zone-empty,
  .zone-filled {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    padding: 1.5rem;
    text-align: center;
    pointer-events: none;
  }
  .zone-empty svg { color: var(--text-tertiary); transition: color 0.2s; }
  .zone:hover .zone-empty svg { color: var(--zone-color); }
  .zone-hint { margin: 0; font-size: 0.875rem; color: var(--text-secondary); }

  .pick-btn,
  .change-btn {
    pointer-events: auto;
    padding: 0.5rem 1.25rem;
    border: none;
    border-radius: 0.375rem;
    font-weight: 600;
    font-size: 0.8rem;
    cursor: pointer;
    transition: all 0.2s;
    background: var(--zone-color);
    color: white;
    opacity: 0.9;
  }

  .pick-btn:hover,
  .change-btn:hover { opacity: 1; transform: translateY(-1px); }

  .change-btn {
    font-size: 0.75rem;
    padding: 0.375rem 1rem;
    background: transparent;
    border: 1.5px solid var(--zone-color);
    color: var(--zone-color);
  }

  .dir-name { margin: 0; font-size: 0.9rem; font-weight: 600; color: var(--text-primary); word-break: break-all; }
  .file-count { margin: 0; font-size: 0.75rem; color: var(--text-secondary); }
  .file-count.warn { color: var(--warning); }
  .missing-warning { margin: 0; font-size: 0.72rem; color: var(--warning); font-weight: 600; }
  .refinefinal-ok { margin: 0; font-size: 0.72rem; color: var(--success); font-weight: 500; }

  .remove-zone {
    position: absolute;
    top: 0.5rem;
    right: 0.5rem;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: none;
    background: var(--bg-hover);
    color: var(--text-tertiary);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s;
    padding: 0;
  }
  .remove-zone:hover { background: var(--error-bg); color: var(--error); }

  .actions { display: flex; justify-content: space-between; align-items: center; }

  .add-genome-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    background: transparent;
    color: var(--text-secondary);
    border: 1.5px solid var(--border-color-dark);
    border-radius: 0.5rem;
    font-size: 0.8rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .add-genome-btn:hover { border-color: var(--accent-primary); color: var(--accent-primary); }

  .submit-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1.75rem;
    background: var(--accent-primary);
    color: white;
    border: none;
    border-radius: 0.5rem;
    font-weight: 600;
    font-size: 0.875rem;
    cursor: pointer;
    transition: all 0.2s;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
  }
  .submit-btn:hover:not(:disabled) { background: var(--accent-hover); transform: translateY(-1px); box-shadow: 0 4px 8px rgba(0, 0, 0, 0.15); }
  .submit-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  .requirement { margin: 0; font-size: 0.72rem; color: var(--text-tertiary); text-align: center; }
</style>