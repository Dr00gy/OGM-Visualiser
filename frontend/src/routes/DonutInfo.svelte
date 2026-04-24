<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { FileData, DonutSegment, MatchedRecord } from '$lib/types';
  import { searchStore } from '$lib/searchStore';
  import ContigPicker from './ContigPicker.svelte';
  import {
    fetchContigs,
    fetchMatchesPage,
    makeDebouncer,
    type ContigAggregate,
    type MatchEntry,
    type SearchType,
  } from '$lib/queryClient';

  // -------------------------------------------------------------------------
  // Props
  // -------------------------------------------------------------------------

  export let files: FileData[] = [];
  export let fileToGenome: number[] = [];
  export let segments: DonutSegment[] = [];
  export let genomeSizes: Map<number, number> = new Map();
  export let totalGenomeSize = 0;
  export let filteredFlowPaths: any[] = [];
  export let showDuplicates = false;

  export let selectedQueryContigId = '';
  export let selectedGenome1 = '';
  export let selectedGenome2 = '';
  export let selectedChromosome = '';
  export let selectedGenomeForChromosome = '';

  export let availableQueryContigIds: number[] = [];
  export let availableGenomes: { value: string; label: string; color: string }[] = [];
  export let availableChromosomes: string[] = [];
  export let clearAllFilters: () => void = () => {};


  export let sessionId: string | null = null;

  export let isQueryable: boolean = false;

  // -------------------------------------------------------------------------
  // Search state (from global store)
  // -------------------------------------------------------------------------

  let overviewSearchQuery = '';
  let matchesSearchQuery = '';
  let overviewSearchType: SearchType = 'contig';
  let matchesSearchType: SearchType = 'contig';

  const unsubscribe = searchStore.subscribe(state => {
    overviewSearchQuery = state.overviewSearchQuery;
    matchesSearchQuery  = state.matchesSearchQuery;
    overviewSearchType  = state.overviewSearchType;
    matchesSearchType   = state.matchesSearchType;
  });

  $: if (overviewSearchQuery !== $searchStore.overviewSearchQuery) {
    searchStore.update(state => ({ ...state, overviewSearchQuery }));
  }
  $: if (matchesSearchQuery !== $searchStore.matchesSearchQuery) {
    searchStore.update(state => ({ ...state, matchesSearchQuery }));
  }
  $: if (overviewSearchType !== $searchStore.overviewSearchType) {
    searchStore.update(state => ({ ...state, overviewSearchType }));
  }
  $: if (matchesSearchType !== $searchStore.matchesSearchType) {
    searchStore.update(state => ({ ...state, matchesSearchType }));
  }

  // -------------------------------------------------------------------------
  // Pagination state
  // -------------------------------------------------------------------------

  let editingOverviewPage = false;
  let editingMatchesPage = false;
  let overviewPageInput = '';
  let matchesPageInput = '';

  let overviewPage = 1;
  const overviewItemsPerPage = 10;

  let matchesPage = 1;
  const matchesItemsPerPage = 10;

  $: overviewSearchQuery, overviewSearchType, overviewPage = 1;
  $: matchesSearchQuery,  matchesSearchType,  matchesPage  = 1;

  // -------------------------------------------------------------------------
  // Overview state
  // -------------------------------------------------------------------------

  type OverviewStat = {
    totalOccurrences: number;
    genomeOccurrences: Map<number, number>;
    chromosomeOccurrences: Map<string, number>;
    maxConfidence: number;
  };
  type OverviewItem = [number, OverviewStat];

  let paginatedOverview: OverviewItem[] = [];
  let overviewTotal = 0;
  $: totalOverviewPages = Math.max(1, Math.ceil(overviewTotal / overviewItemsPerPage));
  let overviewLoading = false;
  let overviewAbort: AbortController | null = null;
  const overviewDebouncer = makeDebouncer(400);


  function aggregateToOverviewItem(agg: ContigAggregate): OverviewItem {
    const genomeMap = new Map<number, number>();

    for (let gi = 0; gi < agg.per_genome.length; gi++) {
      const v = agg.per_genome[gi];
      if (v > 0) genomeMap.set(gi, v);
    }

    const chrMap = new Map<string, number>();
    for (const c of agg.per_chromosome) {
      chrMap.set(`${c.genome_index}-${c.chromosome}`, c.count);
    }

    const stat: OverviewStat = {
      totalOccurrences: agg.total_occurrences,
      genomeOccurrences: genomeMap,
      chromosomeOccurrences: chrMap,
      maxConfidence: agg.max_confidence,
    };
    return [agg.qry_contig_id, stat];
  }

  async function reloadOverview() {
    if (!sessionId || !isQueryable) {
      paginatedOverview = [];
      overviewTotal = 0;
      return;
    }
    if (overviewAbort) overviewAbort.abort();
    overviewAbort = new AbortController();
    const signal = overviewAbort.signal;

    const chipTimer = setTimeout(() => { overviewLoading = true; }, 200);

    try {
      const page = await fetchContigs(sessionId, {
        q: overviewSearchQuery,
        searchType: overviewSearchType,
        page: overviewPage,
        perPage: overviewItemsPerPage,
        signal,
      });
      if (!page) return; // aborted
      overviewTotal = page.total;
      paginatedOverview = page.items.map(aggregateToOverviewItem);
    } catch (err) {
      console.error('Failed to fetch /contigs:', err);
      paginatedOverview = [];
      overviewTotal = 0;
    } finally {
      clearTimeout(chipTimer);
      overviewLoading = false;
    }
  }

  let overviewFirstLoadDone = false;
  let matchesInitialHoldoff = true;

  $: if (sessionId === null || !isQueryable) {
    overviewFirstLoadDone = false;
    matchesInitialHoldoff = true;
  }

  $: if (sessionId && isQueryable) {
    overviewSearchQuery;
    overviewSearchType;
    overviewPage;
    if (!overviewFirstLoadDone) {
      overviewFirstLoadDone = true;
      (async () => {
        await reloadOverview();
        matchesInitialHoldoff = false;
      })();
    } else {
      overviewDebouncer.schedule(() => reloadOverview());
    }
  }

  // -------------------------------------------------------------------------
  // Matches state (backed by /matches endpoint)
  // -------------------------------------------------------------------------

  type MatchItem = {
    qry_contig_id: number;
    records: MatchedRecord[];
    total_record_count: number;
    records_truncated: boolean;
  };

  let paginatedMatches: MatchItem[] = [];
  let matchesTotal = 0;
  $: totalMatchesPages = Math.max(1, Math.ceil(matchesTotal / matchesItemsPerPage));
  let matchesLoading = false;
  let matchesAbort: AbortController | null = null;
  const matchesDebouncer = makeDebouncer(400);

  // -------------------------------------------------------------------------
  // Contig-ID picker
  // -------------------------------------------------------------------------

  function matchEntryToItem(entry: MatchEntry): MatchItem {
    return {
      qry_contig_id: entry.qry_contig_id,
      records: entry.records as unknown as MatchedRecord[],
      total_record_count: entry.total_record_count,
      records_truncated: entry.records_truncated,
    };
  }

  async function reloadMatches() {
    if (!sessionId || !isQueryable) {
      paginatedMatches = [];
      matchesTotal = 0;
      return;
    }
    if (matchesAbort) matchesAbort.abort();
    matchesAbort = new AbortController();
    const signal = matchesAbort.signal;

    const chipTimer = setTimeout(() => { matchesLoading = true; }, 200);

    try {
      const page = await fetchMatchesPage(sessionId, {
        q: matchesSearchQuery,
        searchType: matchesSearchType,
        page: matchesPage,
        perPage: matchesItemsPerPage,
        signal,
      });
      if (!page) return;
      matchesTotal = page.total;
      paginatedMatches = page.items.map(matchEntryToItem);
    } catch (err) {
      console.error('Failed to fetch /matches:', err);
      paginatedMatches = [];
      matchesTotal = 0;
    } finally {
      clearTimeout(chipTimer);
      matchesLoading = false;
    }
  }

  $: if (sessionId && isQueryable && !matchesInitialHoldoff) {
    matchesSearchQuery;
    matchesSearchType;
    matchesPage;
    matchesDebouncer.schedule(() => reloadMatches());
  }

  // -------------------------------------------------------------------------
  // Pagination controls
  // -------------------------------------------------------------------------

  function goToOverviewPage(page: number) {
    overviewPage = Math.max(1, Math.min(page, totalOverviewPages));
  }
  function goToMatchesPage(page: number) {
    matchesPage = Math.max(1, Math.min(page, totalMatchesPages));
  }

  function startEditingOverviewPage() {
    editingOverviewPage = true;
    overviewPageInput = overviewPage.toString();
  }
  function startEditingMatchesPage() {
    editingMatchesPage = true;
    matchesPageInput = matchesPage.toString();
  }

  function submitOverviewPageJump() {
    const pageNum = parseInt(overviewPageInput);
    if (!isNaN(pageNum)) goToOverviewPage(pageNum);
    editingOverviewPage = false;
  }
  function submitMatchesPageJump() {
    const pageNum = parseInt(matchesPageInput);
    if (!isNaN(pageNum)) goToMatchesPage(pageNum);
    editingMatchesPage = false;
  }

  function handleOverviewPageKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') submitOverviewPageJump();
    else if (e.key === 'Escape') editingOverviewPage = false;
  }
  function handleMatchesPageKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') submitMatchesPageJump();
    else if (e.key === 'Escape') editingMatchesPage = false;
  }

  function setOverviewSearchType(type: SearchType) {
    overviewSearchType = type;
    overviewPage = 1;
  }
  function setMatchesSearchType(type: SearchType) {
    matchesSearchType = type;
    matchesPage = 1;
  }

  $: overviewPlaceholder = (() => {
    switch (overviewSearchType) {
      case 'contig': return 'Search by contig ID (number)...';
      case 'chromosome': return 'Search by chromosome (example, "1-2" for genome 1 chromosome 2)...';
      case 'confidence': return 'Search by confidence value (number)...';
      default: return 'Search...';
    }
  })();

  $: matchesPlaceholder = (() => {
    switch (matchesSearchType) {
      case 'contig': return 'Search by contig ID (number)...';
      case 'chromosome': return 'Search by chromosome (number)...';
      case 'confidence': return 'Search by confidence value (number)...';
      default: return 'Search...';
    }
  })();

  // -------------------------------------------------------------------------
  // Cleanup
  // -------------------------------------------------------------------------

  onDestroy(() => {
    unsubscribe();
    if (overviewAbort) overviewAbort.abort();
    if (matchesAbort)  matchesAbort.abort();
    overviewDebouncer.cancel();
    matchesDebouncer.cancel();
  });
</script>

<div class="info">
  <div class="section">
    <h2>Genomes ({files.length})</h2>
    {#each files as file, idx}
      <div class="file-item">
        <div class="color-box" style="background: {file.color}"></div>
        <span class="file-name">{file.name}</span>
        <span class="file-size">{(genomeSizes.get(idx) || 0).toLocaleString()} bp</span>
        <span class="file-pct">({segments[idx]?.percentage}%)</span>
      </div>
    {/each}
  </div>

  {#if isQueryable}
    <div class="section overview-section">
      <h2>
        Query Overview ({overviewTotal.toLocaleString()} {overviewSearchQuery ? 'matching' : 'unique'})
        {#if overviewLoading}
          <span class="loading-chip">loading…</span>
        {/if}
      </h2>
      
      <div class="search-container">
        <div class="search-type-toggle">
          <button 
            class:active={overviewSearchType === 'contig'}
            on:click={() => setOverviewSearchType('contig')}
          >
            Contig ID
          </button>
          <button 
            class:active={overviewSearchType === 'chromosome'}
            on:click={() => setOverviewSearchType('chromosome')}
          >
            Chromosome
          </button>
          <button 
            class:active={overviewSearchType === 'confidence'}
            on:click={() => setOverviewSearchType('confidence')}
          >
            Confidence
          </button>
        </div>
        <div class="search-bar">
          <input
            type="text"
            placeholder={overviewPlaceholder}
            bind:value={overviewSearchQuery}
            class="search-input"
          />
        </div>
      </div>
      
      <div class="overview-list">
        {#each paginatedOverview as [qryId, stat]}
          <div class="overview-item">
            <div class="overview-header">
              <strong>QryContig {qryId}</strong>
              <span class="overview-total">{stat.totalOccurrences} total occurrences</span>
              <span class="overview-confidence">Max conf: {stat.maxConfidence.toFixed(2)}</span>
            </div>
            
            <div class="genome-breakdown">
              <div class="breakdown-label">Per genome:</div>
              {#each Array.from(stat.genomeOccurrences.entries()) as [genomeIdx, count]}
                <span class="genome-badge" style="background: {files[genomeIdx]?.color}20; color: {files[genomeIdx]?.color}; border-color: {files[genomeIdx]?.color}">
                  {files[genomeIdx]?.name}: {count}x
                </span>
              {/each}
            </div>
            
            <div class="chromosome-breakdown">
              <div class="breakdown-label">Per chromosome:</div>
              <div class="chr-grid">
                {#each Array.from(stat.chromosomeOccurrences.entries()) as [chrKey, count]}
                  {@const [genomeIdx, chrNum] = chrKey.split('-').map(Number)}
                  <span class="chr-mini-badge" style="background: {files[genomeIdx]?.color}20; color: {files[genomeIdx]?.color}; border-color: {files[genomeIdx]?.color}">
                    G{genomeIdx} Chr{chrNum}: {count}
                  </span>
                {/each}
              </div>
            </div>
          </div>
        {/each}
      </div>
      
      {#if totalOverviewPages > 1}
        <div class="pagination">
          <button 
            class="page-btn" 
            on:click={() => goToOverviewPage(1)}
            disabled={overviewPage === 1}
          >
            ««
          </button>
          <button 
            class="page-btn" 
            on:click={() => goToOverviewPage(overviewPage - 1)}
            disabled={overviewPage === 1}
          >
            «
          </button>
          
          {#if editingOverviewPage}
            <input
              type="text"
              class="page-input"
              bind:value={overviewPageInput}
              on:keydown={handleOverviewPageKeydown}
              on:blur={submitOverviewPageJump}
              on:focus
            />
          {:else}
            <span 
              class="page-info" 
              on:dblclick={startEditingOverviewPage}
              role="button"
              tabindex="0"
              on:keydown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  startEditingOverviewPage();
                }
              }}
            >
              ({overviewPage} / {totalOverviewPages})
            </span>
          {/if}
          
          <button 
            class="page-btn" 
            on:click={() => goToOverviewPage(overviewPage + 1)}
            disabled={overviewPage === totalOverviewPages}
          >
            »
          </button>
          <button 
            class="page-btn" 
            on:click={() => goToOverviewPage(totalOverviewPages)}
            disabled={overviewPage === totalOverviewPages}
          >
            »»
          </button>
        </div>
      {/if}
    </div>
  {/if}

  <div class="section filters-section">
    <h2>Filters</h2>
    <div class="filters-grid">
      
      <div class="filter-group">
        <label for="query-contig-filter">Query Contig ID:</label>
        <ContigPicker
          id="query-contig-filter"
          options={availableQueryContigIds}
          bind:value={selectedQueryContigId}
          placeholder="Type or pick a contig ID…"
        />
      </div>

      <div class="filter-group">
        <label for="genome1-filter">Genome 1:</label>
        <select id="genome1-filter" bind:value={selectedGenome1}>
          <option value="">All Genomes</option>
          {#each availableGenomes as genome}
            {#if genome.value !== selectedGenome2}
              <option value={genome.value}>{genome.label}</option>
            {/if}
          {/each}
        </select>
      </div>

      <div class="filter-group">
        <label for="genome2-filter">Genome 2 (optional):</label>
        <select id="genome2-filter" bind:value={selectedGenome2}>
          <option value="">Any Genome</option>
          {#each availableGenomes as genome}
            {#if genome.value !== selectedGenome1}
              <option value={genome.value}>{genome.label}</option>
            {/if}
          {/each}
        </select>
      </div>

      <div class="filter-group">
        <label for="genome-chromosome-filter">Genome for Chromosome:</label>
        <select id="genome-chromosome-filter" bind:value={selectedGenomeForChromosome}>
          <option value="">Select Genome</option>
          {#if selectedGenome1 !== ''}
            <option value={selectedGenome1}>
              {availableGenomes.find(g => g.value === selectedGenome1)?.label}
            </option>
          {/if}
          {#if selectedGenome2 !== '' && selectedGenome2 !== selectedGenome1}
            <option value={selectedGenome2}>
              {availableGenomes.find(g => g.value === selectedGenome2)?.label}
            </option>
          {/if}
          {#if selectedGenome1 === '' && selectedGenome2 === ''}
            {#each availableGenomes as genome}
              <option value={genome.value}>{genome.label}</option>
            {/each}
          {/if}
        </select>
      </div>

      <div class="filter-group">
        <label for="chromosome-filter">Chromosome:</label>
        <select id="chromosome-filter" bind:value={selectedChromosome} disabled={!selectedGenomeForChromosome}>
          <option value="">All Chromosomes</option>
          {#each availableChromosomes as chr}
            <option value={chr}>Chr {chr}</option>
          {/each}
        </select>
      </div>

      <div class="filter-group">
        <button on:click={clearAllFilters} class="clear-filters-btn">
          Clear All Filters
        </button>
      </div>
    </div>

    {#if selectedQueryContigId || selectedGenome1 || selectedChromosome}
      <div class="active-filters">
        <h3>Active Filters:</h3>
        <div class="filter-tags">
          {#if selectedQueryContigId}
            <span class="filter-tag">Query Contig: {selectedQueryContigId}</span>
          {/if}
          {#if selectedGenome1}
            <span class="filter-tag">
              Genome: {availableGenomes.find(g => g.value === selectedGenome1)?.label}
              {#if selectedGenome2}
                ↔ {availableGenomes.find(g => g.value === selectedGenome2)?.label}
              {/if}
            </span>
          {/if}
          {#if selectedChromosome && selectedGenomeForChromosome}
            <span class="filter-tag">
              Chromosome {selectedChromosome} on {availableGenomes.find(g => g.value === selectedGenomeForChromosome)?.label}
            </span>
          {/if}
        </div>
      </div>
    {/if}
  </div>

  {#if isQueryable}
    <div class="section">
      <h2>
        Chromosome Matches ({matchesTotal.toLocaleString()} {matchesSearchQuery ? 'matching' : 'unique'})
        {#if matchesLoading}
          <span class="loading-chip">loading…</span>
        {/if}
      </h2>
      
      <div class="search-container">
        <div class="search-type-toggle">
          <button 
            class:active={matchesSearchType === 'contig'}
            on:click={() => setMatchesSearchType('contig')}
          >
            Contig ID
          </button>
          <button 
            class:active={matchesSearchType === 'chromosome'}
            on:click={() => setMatchesSearchType('chromosome')}
          >
            Chromosome
          </button>
          <button 
            class:active={matchesSearchType === 'confidence'}
            on:click={() => setMatchesSearchType('confidence')}
          >
            Confidence
          </button>
        </div>
        <div class="search-bar">
          <input
            type="text"
            placeholder={matchesPlaceholder}
            bind:value={matchesSearchQuery}
            class="search-input"
          />
        </div>
      </div>
      
      <div class="match-list">
        {#each paginatedMatches as match}
          <div class="match-item">
            <div class="match-header">
              <strong>QryContig {match.qry_contig_id}</strong>
              <span class="occurrence-count">
                {match.total_record_count} occurrence{match.total_record_count !== 1 ? 's' : ''}
              </span>
            </div>
            <div class="occurrence-list">
              {#each match.records as record}
                {@const genomeIdx = fileToGenome[record.file_index] ?? record.file_index}
                <div class="occurrence">
                  <span class="file-badge" style="background: {files[genomeIdx]?.color}20; color: {files[genomeIdx]?.color}; border-color: {files[genomeIdx]?.color}">
                    {files[genomeIdx]?.name}
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
      
      {#if totalMatchesPages > 1}
        <div class="pagination">
          <button 
            class="page-btn" 
            on:click={() => goToMatchesPage(1)}
            disabled={matchesPage === 1}
          >
            ««
          </button>
          <button 
            class="page-btn" 
            on:click={() => goToMatchesPage(matchesPage - 1)}
            disabled={matchesPage === 1}
          >
            «
          </button>
          
          {#if editingMatchesPage}
            <input
              type="text"
              class="page-input"
              bind:value={matchesPageInput}
              on:keydown={handleMatchesPageKeydown}
              on:blur={submitMatchesPageJump}
              on:focus
            />
          {:else}
            <span 
              class="page-info" 
              on:dblclick={startEditingMatchesPage}
              role="button"
              tabindex="0"
              on:keydown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  startEditingMatchesPage();
                }
              }}
            >
              ({matchesPage} / {totalMatchesPages})
            </span>
          {/if}
          
          <button 
            class="page-btn" 
            on:click={() => goToMatchesPage(matchesPage + 1)}
            disabled={matchesPage === totalMatchesPages}
          >
            »
          </button>
          <button 
            class="page-btn" 
            on:click={() => goToMatchesPage(totalMatchesPages)}
            disabled={matchesPage === totalMatchesPages}
          >
            »»
          </button>
        </div>
      {/if}
    </div>
  {/if}

  <div class="section debug-info">
    <h2>Debug Info</h2>
    <div class="debug-item">
      <strong>Total Genome Size:</strong> {totalGenomeSize.toLocaleString()} bp
    </div>
    <div class="debug-item">
      <strong>Flow Paths:</strong> {filteredFlowPaths.length} {showDuplicates ? '(self-flow)' : '(cross-genome)'}
    </div>
    <div class="debug-item">
      <strong>Show Self-Flow:</strong> {showDuplicates ? 'ON' : 'OFF'}
    </div>
    <div class="debug-item">
      <strong>Active Filters:</strong> 
      {selectedQueryContigId ? 'QueryContig ' + selectedQueryContigId + ' ' : ''}
      {selectedGenome1 ? 'Genome1:' + selectedGenome1 + ' ' : ''}
      {selectedGenome2 ? 'Genome2:' + selectedGenome2 + ' ' : ''}
      {selectedChromosome ? 'Chr:' + selectedChromosome + ' ' : ''}
      {!selectedQueryContigId && !selectedGenome1 && !selectedChromosome ? 'None' : ''}
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

  /* Shown under the occurrence list when the server truncated the
     records (more than MAX_RECORDS_PER_MATCH_ENTRY on the backend).
     Subtle — not an error, just an informational "there's more" tag. */
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