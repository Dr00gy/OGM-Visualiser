<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { FileData, DonutSeg } from '$lib/types';
  import { searchStore } from '$lib/searchStore';
  import SeqPicker from './SeqPicker.svelte';
  import {
    fetchSeqs,
    fetchMatchesPage,
    makeDebouncer,
    type SequenceAggregate,
    type MatchEntry,
    type SearchType,
  } from '$lib/queryClient';

  export let files: FileData[] = [];
  export let fileToGen: number[] = [];
  export let segments: DonutSeg[] = [];
  export let genSizes: Map<number, number> = new Map();
  export let totGenSize = 0;
  export let fltFlowPaths: any[] = [];
  export let showDups = false;

  export let selSeqId = '';
  export let selGen1 = '';
  export let selGen2 = '';
  export let selChr = '';
  export let selGenForChr = '';

  export let availSeqIds: number[] = [];
  export let availGens: { value: string; label: string; color: string }[] = [];
  export let availChrs: string[] = [];
  export let clearAllFlts: () => void = () => {};

  export let sessId: string | null = null;
  export let isQueryable: boolean = false;

  let ovQry = '';
  let mtcQry = '';
  let ovType: SearchType = 'sequence';
  let mtcType: SearchType = 'sequence';

  const unsub = searchStore.subscribe(state => {
    ovQry  = state.ovQry;
    mtcQry = state.mtcQry;
    ovType = state.ovType;
    mtcType = state.mtcType;
  });

  $: searchStore.update(s => (
    s.ovQry === ovQry && s.mtcQry === mtcQry && s.ovType === ovType && s.mtcType === mtcType
      ? s : { ...s, ovQry, mtcQry, ovType, mtcType }
  ));

  let editOvPage = false;
  let editMtcPage = false;
  let ovPageInput = '';
  let mtcPageInput = '';

  let ovPage = 1;
  const OV_PER_PAGE = 10;

  let mtcPage = 1;
  const MTC_PER_PAGE = 10;

  $: ovQry, ovType, ovPage = 1;
  $: mtcQry, mtcType, mtcPage = 1;

  let ovItems: SequenceAggregate[] = [];
  let ovTotal = 0;
  $: totOvPages = Math.max(1, Math.ceil(ovTotal / OV_PER_PAGE));
  let ovLoading = false;
  let ovAbort: AbortController | null = null;
  const ovDeb = makeDebouncer(400);

  async function reloadOv() {
    if (!sessId || !isQueryable) {
      ovItems = []; ovTotal = 0;
      return;
    }
    if (ovAbort) ovAbort.abort();
    ovAbort = new AbortController();
    const signal = ovAbort.signal;
    const chipTimer = setTimeout(() => { ovLoading = true; }, 200);

    try {
      const page = await fetchSeqs(sessId, {
        q: ovQry, searchType: ovType, page: ovPage, perPage: OV_PER_PAGE, signal,
      });
      if (!page) return;
      ovTotal = page.total;
      ovItems = page.items;
    } catch (err) {
      console.error('Failed to fetch /sequences:', err);
      ovItems = []; ovTotal = 0;
    } finally {
      clearTimeout(chipTimer);
      ovLoading = false;
    }
  }

  let ovFirstLoaded = false;
  let mtcHoldoff = true;

  $: if (sessId === null || !isQueryable) {
    ovFirstLoaded = false;
    mtcHoldoff = true;
  }

  $: if (sessId && isQueryable) {
    ovQry; ovType; ovPage;
    if (!ovFirstLoaded) {
      ovFirstLoaded = true;
      (async () => { await reloadOv(); mtcHoldoff = false; })();
    } else {
      ovDeb.schedule(() => reloadOv());
    }
  }

  let mtcItems: MatchEntry[] = [];
  let mtcTotal = 0;
  $: totMtcPages = Math.max(1, Math.ceil(mtcTotal / MTC_PER_PAGE));
  let mtcLoading = false;
  let mtcAbort: AbortController | null = null;
  const mtcDeb = makeDebouncer(400);

  async function reloadMtc() {
    if (!sessId || !isQueryable) {
      mtcItems = []; mtcTotal = 0;
      return;
    }
    if (mtcAbort) mtcAbort.abort();
    mtcAbort = new AbortController();
    const signal = mtcAbort.signal;
    const chipTimer = setTimeout(() => { mtcLoading = true; }, 200);

    try {
      const page = await fetchMatchesPage(sessId, {
        q: mtcQry, searchType: mtcType, page: mtcPage, perPage: MTC_PER_PAGE, signal,
      });
      if (!page) return;
      mtcTotal = page.total;
      mtcItems = page.items;
    } catch (err) {
      console.error('Failed to fetch /matches:', err);
      mtcItems = []; mtcTotal = 0;
    } finally {
      clearTimeout(chipTimer);
      mtcLoading = false;
    }
  }

  $: if (sessId && isQueryable && !mtcHoldoff) {
    mtcQry; mtcType; mtcPage;
    mtcDeb.schedule(() => reloadMtc());
  }

  function goToOvPage(page: number) { ovPage = Math.max(1, Math.min(page, totOvPages)); }
  function goToMtcPage(page: number) { mtcPage = Math.max(1, Math.min(page, totMtcPages)); }

  function startEditOvPage() { editOvPage = true; ovPageInput = ovPage.toString(); }
  function startEditMtcPage() { editMtcPage = true; mtcPageInput = mtcPage.toString(); }

  function submitOvJump() {
    const n = parseInt(ovPageInput);
    if (!isNaN(n)) goToOvPage(n);
    editOvPage = false;
  }
  function submitMtcJump() {
    const n = parseInt(mtcPageInput);
    if (!isNaN(n)) goToMtcPage(n);
    editMtcPage = false;
  }

  function onOvPageKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') submitOvJump();
    else if (e.key === 'Escape') editOvPage = false;
  }
  function onMtcPageKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') submitMtcJump();
    else if (e.key === 'Escape') editMtcPage = false;
  }

  function setOvType(type: SearchType) { ovType = type; ovPage = 1; }
  function setMtcType(type: SearchType) { mtcType = type; mtcPage = 1; }

  const PLACEHOLDER_BY_TYPE: Record<SearchType, string> = {
    sequence:   'Search by sequence ID (number)...',
    chromosome: 'Search by chromosome (number)...',
    confidence: 'Search by confidence value (number)...',
  };

  $: ovPlaceholder = ovType === 'chromosome'
    ? 'Search by chromosome (example, "1-2" for genome 1 chromosome 2)...'
    : PLACEHOLDER_BY_TYPE[ovType];
  $: mtcPlaceholder = PLACEHOLDER_BY_TYPE[mtcType];

  onDestroy(() => {
    unsub();
    if (ovAbort) ovAbort.abort();
    if (mtcAbort) mtcAbort.abort();
    ovDeb.cancel();
    mtcDeb.cancel();
  });
</script>

<div class="info">
  <div class="section">
    <h2>Genomes ({files.length})</h2>
    {#each files as file, idx}
      <div class="file-item">
        <div class="color-box" style="background: {file.color}"></div>
        <span class="file-name">{file.name}</span>
        <span class="file-size">{(genSizes.get(idx) || 0).toLocaleString()} bp</span>
        <span class="file-pct">({segments[idx]?.pct}%)</span>
      </div>
    {/each}
  </div>

  {#if isQueryable}
    <div class="section overview-section">
      <h2>
        Query Overview ({ovTotal.toLocaleString()} {ovQry ? 'matching' : 'unique'})
        {#if ovLoading}
          <span class="loading-chip">loading…</span>
        {/if}
      </h2>

      <div class="search-container">
        <div class="search-type-toggle">
          <button class:active={ovType === 'sequence'} on:click={() => setOvType('sequence')}>
            Sequence ID
          </button>
          <button class:active={ovType === 'chromosome'} on:click={() => setOvType('chromosome')}>
            Chromosome
          </button>
          <button class:active={ovType === 'confidence'} on:click={() => setOvType('confidence')}>
            Confidence
          </button>
        </div>
        <div class="search-bar">
          <input
            type="text"
            placeholder={ovPlaceholder}
            bind:value={ovQry}
            class="search-input"
          />
        </div>
      </div>

      <div class="overview-list">
        {#each ovItems as agg}
          <div class="overview-item">
            <div class="overview-header">
              <strong>QryContig {agg.qry_contig_id}</strong>
              <span class="overview-total">{agg.total_occurrences} total occurrences</span>
              <span class="overview-confidence">Max conf: {agg.max_confidence.toFixed(2)}</span>
            </div>

            <div class="genome-breakdown">
              <div class="breakdown-label">Per genome:</div>
              {#each agg.per_genome as count, genIdx}
                {#if count > 0}
                  <span class="genome-badge" style="background: {files[genIdx]?.color}20; color: {files[genIdx]?.color}; border-color: {files[genIdx]?.color}">
                    {files[genIdx]?.name}: {count}x
                  </span>
                {/if}
              {/each}
            </div>

            <div class="chromosome-breakdown">
              <div class="breakdown-label">Per chromosome:</div>
              <div class="chr-grid">
                {#each agg.per_chromosome as c}
                  <span class="chr-mini-badge" style="background: {files[c.genome_index]?.color}20; color: {files[c.genome_index]?.color}; border-color: {files[c.genome_index]?.color}">
                    G{c.genome_index} Chr{c.chromosome}: {c.count}
                  </span>
                {/each}
              </div>
            </div>
          </div>
        {/each}
      </div>

      {#if totOvPages > 1}
        <div class="pagination">
          <button class="page-btn" on:click={() => goToOvPage(1)} disabled={ovPage === 1}>««</button>
          <button class="page-btn" on:click={() => goToOvPage(ovPage - 1)} disabled={ovPage === 1}>«</button>

          {#if editOvPage}
            <input
              type="text"
              class="page-input"
              bind:value={ovPageInput}
              on:keydown={onOvPageKeydown}
              on:blur={submitOvJump}
              on:focus
            />
          {:else}
            <span
              class="page-info"
              on:dblclick={startEditOvPage}
              role="button"
              tabindex="0"
              on:keydown={(e) => { if (e.key === 'Enter' || e.key === ' ') startEditOvPage(); }}
            >
              ({ovPage} / {totOvPages})
            </span>
          {/if}

          <button class="page-btn" on:click={() => goToOvPage(ovPage + 1)} disabled={ovPage === totOvPages}>»</button>
          <button class="page-btn" on:click={() => goToOvPage(totOvPages)} disabled={ovPage === totOvPages}>»»</button>
        </div>
      {/if}
    </div>
  {/if}

  <div class="section filters-section">
    <h2>Filters</h2>
    <div class="filters-grid">

      <div class="filter-group">
        <label for="query-seq-filter">Query Sequence ID:</label>
        <SeqPicker
          id="query-seq-filter"
          options={availSeqIds}
          bind:value={selSeqId}
          placeholder="Type or pick a sequence ID…"
        />
      </div>

      <div class="filter-group">
        <label for="genome1-filter">Genome 1:</label>
        <select id="genome1-filter" bind:value={selGen1}>
          <option value="">All Genomes</option>
          {#each availGens as gen}
            {#if gen.value !== selGen2}
              <option value={gen.value}>{gen.label}</option>
            {/if}
          {/each}
        </select>
      </div>

      <div class="filter-group">
        <label for="genome2-filter">Genome 2 (optional):</label>
        <select id="genome2-filter" bind:value={selGen2}>
          <option value="">Any Genome</option>
          {#each availGens as gen}
            {#if gen.value !== selGen1}
              <option value={gen.value}>{gen.label}</option>
            {/if}
          {/each}
        </select>
      </div>

      <div class="filter-group">
        <label for="genome-chromosome-filter">Genome for Chromosome:</label>
        <select id="genome-chromosome-filter" bind:value={selGenForChr}>
          <option value="">Select Genome</option>
          {#if selGen1 !== ''}
            <option value={selGen1}>
              {availGens.find(g => g.value === selGen1)?.label}
            </option>
          {/if}
          {#if selGen2 !== '' && selGen2 !== selGen1}
            <option value={selGen2}>
              {availGens.find(g => g.value === selGen2)?.label}
            </option>
          {/if}
          {#if selGen1 === '' && selGen2 === ''}
            {#each availGens as gen}
              <option value={gen.value}>{gen.label}</option>
            {/each}
          {/if}
        </select>
      </div>

      <div class="filter-group">
        <label for="chromosome-filter">Chromosome:</label>
        <select id="chromosome-filter" bind:value={selChr} disabled={!selGenForChr}>
          <option value="">All Chromosomes</option>
          {#each availChrs as chr}
            <option value={chr}>Chr {chr}</option>
          {/each}
        </select>
      </div>

      <div class="filter-group">
        <button on:click={clearAllFlts} class="clear-filters-btn">
          Clear All Filters
        </button>
      </div>
    </div>

    {#if selSeqId || selGen1 || selChr}
      <div class="active-filters">
        <h3>Active Filters:</h3>
        <div class="filter-tags">
          {#if selSeqId}
            <span class="filter-tag">Query Sequence: {selSeqId}</span>
          {/if}
          {#if selGen1}
            <span class="filter-tag">
              Genome: {availGens.find(g => g.value === selGen1)?.label}
              {#if selGen2}
                ↔ {availGens.find(g => g.value === selGen2)?.label}
              {/if}
            </span>
          {/if}
          {#if selChr && selGenForChr}
            <span class="filter-tag">
              Chromosome {selChr} on {availGens.find(g => g.value === selGenForChr)?.label}
            </span>
          {/if}
        </div>
      </div>
    {/if}
  </div>

  {#if isQueryable}
    <div class="section">
      <h2>
        Chromosome Matches ({mtcTotal.toLocaleString()} {mtcQry ? 'matching' : 'unique'})
        {#if mtcLoading}
          <span class="loading-chip">loading…</span>
        {/if}
      </h2>

      <div class="search-container">
        <div class="search-type-toggle">
          <button class:active={mtcType === 'sequence'} on:click={() => setMtcType('sequence')}>
            Sequence ID
          </button>
          <button class:active={mtcType === 'chromosome'} on:click={() => setMtcType('chromosome')}>
            Chromosome
          </button>
          <button class:active={mtcType === 'confidence'} on:click={() => setMtcType('confidence')}>
            Confidence
          </button>
        </div>
        <div class="search-bar">
          <input
            type="text"
            placeholder={mtcPlaceholder}
            bind:value={mtcQry}
            class="search-input"
          />
        </div>
      </div>

      <div class="match-list">
        {#each mtcItems as match}
          <div class="match-item">
            <div class="match-header">
              <strong>QryContig {match.qry_contig_id}</strong>
              <span class="occurrence-count">
                {match.total_record_count} occurrence{match.total_record_count !== 1 ? 's' : ''}
              </span>
            </div>
            <div class="occurrence-list">
              {#each match.records as record}
                {@const genIdx = fileToGen[record.file_index] ?? record.file_index}
                <div class="occurrence">
                  <span class="file-badge" style="background: {files[genIdx]?.color}20; color: {files[genIdx]?.color}; border-color: {files[genIdx]?.color}">
                    {files[genIdx]?.name}
                  </span>
                  <span class="chr-info">Chr {record.ref_contig_id}</span>
                  <span class="orientation-badge" class:plus={record.orientation === '+'} class:minus={record.orientation === '-'}>
                    {record.orientation}
                  </span>
                  <span class="confidence-value">conf: {record.confidence.toFixed(2)}</span>
                </div>
              {/each}
              {#if match.records_truncated}
                <div class="records-truncated-hint">
                  Showing first {match.records.length} of {match.total_record_count} records.
                </div>
              {/if}
            </div>
          </div>
        {/each}
      </div>

      {#if totMtcPages > 1}
        <div class="pagination">
          <button class="page-btn" on:click={() => goToMtcPage(1)} disabled={mtcPage === 1}>««</button>
          <button class="page-btn" on:click={() => goToMtcPage(mtcPage - 1)} disabled={mtcPage === 1}>«</button>

          {#if editMtcPage}
            <input
              type="text"
              class="page-input"
              bind:value={mtcPageInput}
              on:keydown={onMtcPageKeydown}
              on:blur={submitMtcJump}
              on:focus
            />
          {:else}
            <span
              class="page-info"
              on:dblclick={startEditMtcPage}
              role="button"
              tabindex="0"
              on:keydown={(e) => { if (e.key === 'Enter' || e.key === ' ') startEditMtcPage(); }}
            >
              ({mtcPage} / {totMtcPages})
            </span>
          {/if}

          <button class="page-btn" on:click={() => goToMtcPage(mtcPage + 1)} disabled={mtcPage === totMtcPages}>»</button>
          <button class="page-btn" on:click={() => goToMtcPage(totMtcPages)} disabled={mtcPage === totMtcPages}>»»</button>
        </div>
      {/if}
    </div>
  {/if}

  <div class="section debug-info">
    <h2>Debug Info</h2>
    <div class="debug-item">
      <strong>Total Genome Size:</strong> {totGenSize.toLocaleString()} bp
    </div>
    <div class="debug-item">
      <strong>Flow Paths:</strong> {fltFlowPaths.length} {showDups ? '(self-flow)' : '(cross-genome)'}
    </div>
    <div class="debug-item">
      <strong>Show Self-Flow:</strong> {showDups ? 'ON' : 'OFF'}
    </div>
    <div class="debug-item">
      <strong>Active Filters:</strong>
      {selSeqId ? 'QuerySeq ' + selSeqId + ' ' : ''}
      {selGen1 ? 'Genome1:' + selGen1 + ' ' : ''}
      {selGen2 ? 'Genome2:' + selGen2 + ' ' : ''}
      {selChr ? 'Chr:' + selChr + ' ' : ''}
      {!selSeqId && !selGen1 && !selChr ? 'None' : ''}
    </div>
  </div>
</div>

<style>
  .info { flex: 1; min-width: 280px;}

  .section {
    margin-bottom: 1.5rem;
    padding: clamp(0.75rem, 1.5vw, 1rem);
    background: var(--bg-secondary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
    max-width: 100%;
    overflow: hidden;
  }

  h2 {
    font-size: clamp(0.95rem, 1.4vw, 1rem);
    font-weight: 600;
    margin-bottom: 0.75rem;
    color: var(--text-primary);
  }

  .loading-chip {
    margin-left: 0.5rem;
    padding: 0.125rem 0.5rem;
    background: var(--accent-light);
    color: var(--accent-primary);
    border-radius: 0.375rem;
    font-size: 0.7rem;
    font-weight: 500;
    vertical-align: middle;
  }

  h3 {
    font-size: clamp(0.85rem, 1.3vw, 0.95rem);
    font-weight: 600;
    margin-bottom: 0.5rem;
    color: var(--text-primary);
  }

  .overview-section {
    background: var(--bg-secondary);
    border-color: var(--border-color);
  }
  .overview-list { max-height: 500px; overflow-y: auto; }
  .overview-item {
    margin-bottom: 1rem;
    padding: 1rem;
    background: var(--bg-primary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
  }
  .overview-header {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    margin-bottom: 0.75rem;
    flex-wrap: wrap;
  }
  .overview-header strong {
    color: var(--accent-primary);
    font-size: 0.875rem;
  }

  .overview-total {
    padding: 0.25rem 0.5rem;
    background: var(--accent-light);
    color: var(--accent-primary);
    border-radius: 0.25rem;
    font-size: 0.7rem;
    font-weight: 600;
    white-space: nowrap;
  }

  .overview-confidence {
    padding: 0.25rem 0.5rem;
    background: rgba(16, 185, 129, 0.2);
    color: var(--success);
    border-radius: 0.25rem;
    font-size: 0.7rem;
    font-weight: 600;
    white-space: nowrap;
  }

  .genome-breakdown,
  .chromosome-breakdown {
    margin-bottom: 0.5rem;
    font-size: 0.75rem;
  }
  .breakdown-label {
    font-weight: 500;
    color: var(--text-secondary);
    margin-bottom: 0.25rem;
  }
  .genome-breakdown {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    align-items: center;
  }

  .genome-badge {
    padding: 0.25rem 0.5rem;
    border-radius: 0.25rem;
    font-size: 0.7rem;
    font-weight: 600;
    border: 1px solid;
    white-space: nowrap;
  }

  .chr-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin-top: 0.25rem;
  }
  .chr-mini-badge {
    padding: 0.125rem 0.375rem;
    border-radius: 0.25rem;
    font-size: 0.65rem;
    font-weight: 500;
    border: 1px solid;
    white-space: nowrap;
  }

  .pagination {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    justify-content: center;
    margin-top: 1rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border-color);
  }

  .page-btn {
    padding: 0.5rem 0.75rem;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 0.375rem;
    color: var(--text-primary);
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
    min-width: 2.5rem;
  }

  .page-btn:hover:not(:disabled) {
    background: var(--accent-primary);
    color: white;
    border-color: var(--accent-primary);
  }

  .page-btn:disabled {
    background: var(--bg-hover);
    color: var(--text-tertiary);
    cursor: not-allowed;
    opacity: 0.5;
  }

  .page-info {
    padding: 0.5rem 0.75rem;
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--text-primary);
    cursor: pointer;
    user-select: none;
    transition: background 0.2s;
    border-radius: 0.375rem;
  }

  .page-info:hover {
    background: var(--bg-hover);
  }

  .page-input {
    width: 4rem;
    padding: 0.5rem;
    text-align: center;
    font-size: 0.875rem;
    font-weight: 500;
    border: 2px solid var(--accent-primary);
    border-radius: 0.375rem;
    background: var(--bg-primary);
    color: var(--text-primary);
  }

  .page-input:focus {
    outline: none;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  .search-container { margin-bottom: 1rem; }
  .search-type-toggle {
    display: flex;
    gap: 0.25rem;
    margin-bottom: 0.5rem;
    flex-wrap: wrap;
  }

  .search-type-toggle button {
    padding: 0.375rem 0.75rem;
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    color: var(--text-secondary);
    border-radius: 0.375rem;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
    white-space: nowrap;
  }

  .search-type-toggle button:hover {
    background: var(--bg-hover);
    border-color: var(--accent-primary);
  }

  .search-type-toggle button.active {
    background: var(--accent-primary);
    border-color: var(--accent-primary);
    color: white;
  }

  .search-bar {
    position: relative;
    max-width: 100%;
    width: 100%;
    box-sizing: border-box;
    z-index: 1;
  }

  .search-input {
    width: 100%;
    padding: 0.625rem 0.75rem;
    font-size: 0.875rem;
    border: 1px solid var(--border-color-dark);
    border-radius: 0.375rem;
    background: var(--bg-primary);
    color: var(--text-primary);
    transition: border-color 0.2s;
    box-sizing: border-box;
    position: relative;
    z-index: 1;
  }

  .search-input:focus {
    outline: none;
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
    z-index: 2;
  }
  .search-input::placeholder { color: var(--text-tertiary); }

  .filters-section {
    background: var(--bg-accent);
    border-color: var(--border-color);
  }

  .filters-grid {
    display: grid;
    gap: 1rem;
    grid-template-columns: 1fr 1fr;
    margin-bottom: 1rem;
  }

  .filter-group {
    display: flex;
    gap: 0.25rem;
    flex-direction: column;
  }

  .filter-group label {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--text-primary);
  }

  .filter-group select {
    padding: 0.5rem;
    border: 1px solid var(--border-color-dark);
    border-radius: 0.375rem;
    font-size: 0.8rem;
    background: var(--bg-primary);
    color: var(--text-primary);
    width: 100%;
    box-sizing: border-box;
  }

  .filter-group select:disabled {
    background: var(--bg-hover);
    color: var(--text-tertiary);
    cursor: not-allowed;
  }

  .clear-filters-btn {
    padding: 0.5rem 1rem;
    background: var(--accent-primary);
    color: white;
    border: none;
    border-radius: 0.375rem;
    font-size: 0.8rem;
    cursor: pointer;
    margin-top: 1.25rem;
    width: 100%;
    transition: background 0.2s;
  }

  .clear-filters-btn:hover { background: var(--accent-hover); }

  .active-filters {
    margin-top: 1rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border-color);
  }

  .filter-tags {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .filter-tag {
    padding: 0.25rem 0.5rem;
    background: var(--accent-primary);
    color: white;
    border-radius: 0.25rem;
    font-size: 0.7rem;
    font-weight: 500;
    white-space: nowrap;
  }

  .file-item {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    margin-bottom: 0.5rem;
    font-size: 0.875rem;
    flex-wrap: nowrap;
    min-width: 0;
  }

  .color-box {
    width: 1rem;
    height: 1rem;
    border-radius: 0.25rem;
    flex-shrink: 0;
  }

  .file-name {
    font-weight: 500;
    color: var(--text-primary);
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-size,
  .file-pct {
    flex-shrink: 0;
    color: var(--text-secondary);
    font-size: 0.75rem;
    white-space: nowrap;
  }

  .match-list { max-height: 400px; overflow-y: auto; }
  .match-item {
    font-size: 0.8rem;
    margin-bottom: 0.75rem;
    padding: 0.75rem;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
  }

  .match-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.5rem;
  }

  .match-header strong {
    color: var(--accent-primary);
  }

  .occurrence-count {
    font-size: 0.7rem;
    color: var(--text-secondary);
  }

  .file-badge {
    padding: 0.25rem 0.5rem;
    margin-right: 0.5rem;
    border-radius: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    border: 1px solid;
    white-space: nowrap;
  }

  .chr-info {
    margin-right: 0.5rem;
    color: var(--text-secondary);
  }

  .orientation-badge {
    padding: 0.125rem 0.375rem;
    border-radius: 0.25rem;
    font-size: 0.7rem;
    font-weight: 600;
    margin-right: 0.5rem;
  }

  .orientation-badge.plus {
    background: rgba(16, 185, 129, 0.2);
    color: var(--success);
  }

  .orientation-badge.minus {
    background: rgba(239, 68, 68, 0.2);
    color: var(--error);
  }

  .confidence-value {
    font-size: 0.7rem;
    color: var(--text-secondary);
  }

  .occurrence { margin-top: 0.8rem; }
  .occurrence-list { margin-bottom: 0.5rem; }

  /* Shown when the server truncated records (more on the backend). */
  .records-truncated-hint {
    margin-top: 0.375rem;
    padding: 0.25rem 0.5rem;
    color: var(--text-tertiary);
    font-size: 0.7rem;
    font-style: italic;
    text-align: center;
  }

  .debug-info {
    background: var(--error-bg);
    border-color: var(--error-border);
  }

  .debug-item {
    font-size: 0.75rem;
    margin-bottom: 0.25rem;
    color: var(--text-primary);
  }

  .debug-item strong {
    color: var(--text-primary);
  }

  @media (max-width: 1024px) {
    .overview-list {
      max-height: 420px;
    }
  }

  @media (max-width: 768px) {
    .filters-grid {
      grid-template-columns: 1fr;
    }
    .file-item {
      grid-template-columns: auto 1fr;
      grid-auto-rows: auto;
      row-gap: 0.25rem;
    }
    .file-size,
    .file-pct {
      grid-column: 2 / -1;
    }
    .match-list {
      max-height: 360px;
    }
    .search-type-toggle {
      gap: 0.125rem;
    }
    .search-type-toggle button {
      padding: 0.25rem 0.5rem;
      font-size: 0.7rem;
    }
  }

  @media (max-width: 520px) {
    .section {
      padding: 0.75rem;
    }
    h2 { font-size: 0.9rem; }
    h3 { font-size: 0.85rem; }
    .genome-badge,
    .file-badge,
    .filter-tag {
      font-size: 0.7rem;
    }
    .overview-header strong {
      font-size: 0.8rem;
    }
    .overview-list {
      max-height: 320px;
    }
    .page-btn {
      padding: 0.4rem 0.6rem;
      font-size: 0.8rem;
      min-width: 2rem;
    }
    .page-info {
      font-size: 0.8rem;
      padding: 0.4rem 0.6rem;
    }
    .search-type-toggle {
      gap: 0.25rem;
      flex-direction: column;
    }
    .search-type-toggle button {
      padding: 0.375rem 0.5rem;
      font-size: 0.75rem;
    }
  }

  @media (max-width: 380px) {
    .overview-total,
    .overview-confidence {
      font-size: 0.65rem;
      padding: 0.2rem 0.4rem;
    }
    .filter-group select {
      font-size: 0.75rem;
      padding: 0.45rem;
    }
    .clear-filters-btn {
      font-size: 0.75rem;
    }
  }

  @media (max-width: 300px) {
    .file-item {
      flex-wrap: wrap;
      gap: 0.25rem 0.5rem;
    }
    .file-name {
      flex: 1 1 100%;
    }
  }
</style>