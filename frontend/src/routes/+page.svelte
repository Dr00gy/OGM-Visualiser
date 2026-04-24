<script lang="ts">
  import { onMount } from 'svelte';
  import {
    processMatchStream,
    type BackendMatch,
    type ChromosomeInfo,
  } from '$lib/bincodeDecoder';
  import { darkMode } from '$lib/darkModeStore';
  import { donutFilterState } from '$lib/filterStateStore';
  import { resetSearchStore } from '$lib/searchStore';
  import FileUpload from './FileUpload.svelte';
  import ErrorBanner from './ErrorBanner.svelte';
  import LoadingSpinner from './LoadingSpinner.svelte';
  import TabNav from './TabNav.svelte';
  import DarkModeToggle from './DarkModeToggle.svelte';
  import DonutVisualisation from './DonutVisualisation.svelte';
  import DisplayControls from './DisplayControls.svelte';
  let AreaAnalysis: any = null;
  let loadingAreaAnalysis = false;

  interface GenomeData {
    name: string;
    fileCount: number;
    rows: number;
    color: string;
  }

  const GENOME_COLORS = ['#3b82f6', '#10b981', '#f59e0b'];
  let genomes: GenomeData[] = [];
  let fileToGenome: number[] = [];
  let matches: BackendMatch[] = [];
  let genomeRowCounts: number[] = [];//TODO:
  let chromosomeInfo: ChromosomeInfo[][] = [];

  let isLoading = false;
  let loadingText = 'Initialising stream...';

  let sessionId: string | null = null;
  let isStreaming = false;//TODO:
  let error = '';

  let matchCount = 0;
  let recordCount = 0;
  let distinctContigCount = 0;
  let abortController: AbortController | null = null;
  let showDuplicates = false;

  let activeTab: 'visualization' | 'analysis' = 'visualization';
  let hasUploadedFiles = false;
  let hasChromosomeInfo = false;
  let streamComplete = false;

  onMount(() => {
    document.documentElement.classList.toggle('dark', $darkMode);
  });

  async function loadAreaAnalysis() {
    if (AreaAnalysis || loadingAreaAnalysis) return;
    loadingAreaAnalysis = true;
    try {
      const module = await import('./AreaAnalysis.svelte');
      AreaAnalysis = module.default;
    } catch (err) {
      console.error('Failed to load AreaAnalysis:', err);
    } finally {
      loadingAreaAnalysis = false;
    }
  }

  $: if (activeTab === 'analysis' && streamComplete) {
    loadAreaAnalysis();
  }

  async function handleFileUpload(genomeGroups: { contigFiles: File[]; refineFinalFile: File; dirName: string }[]) {
    if (!genomeGroups || genomeGroups.length < 2) return;

    if (abortController) {
      abortController.abort();
    }

    if (sessionId) {
      const oldId = sessionId;
      void fetch(`http://localhost:8080/api/session/${oldId}`, {
        method: 'DELETE',
      }).catch(() => { /* ignore cleanup failures */ });
      sessionId = null;
    }

    abortController = new AbortController();
    isLoading = true;
    isStreaming = true;
    streamComplete = false;
    error = '';
    matches = [];
    chromosomeInfo = [];
    matchCount = 0;
    recordCount = 0;
    distinctContigCount = 0;
    hasUploadedFiles = false;
    hasChromosomeInfo = false;

    donutFilterState.reset();
    resetSearchStore();

    genomes = genomeGroups.map((g, i) => ({
      name: g.dirName,
      fileCount: g.contigFiles.length,
      rows: 0,
      color: GENOME_COLORS[i],
    }));

    genomeRowCounts = new Array(genomes.length).fill(0);

    fileToGenome = genomeGroups.flatMap((g, gi) =>
      Array.from({ length: g.contigFiles.length }, () => gi)
    );

    try {
      loadingText = 'Creating session...';
      const sessionResp = await fetch('http://localhost:8080/api/session', {
        method: 'POST',
        signal: abortController.signal,
      });
      if (!sessionResp.ok) {
        throw new Error(`Session create failed: ${sessionResp.status} ${sessionResp.statusText}`);
      }
      const { session_id } = await sessionResp.json() as { session_id: string };
      sessionId = session_id;

      type UploadJob = { fieldName: string; file: File };
      const jobs: UploadJob[] = [];
      genomeGroups.forEach((group, gi) => {
        jobs.push({ fieldName: `g${gi}_r`, file: group.refineFinalFile });
        group.contigFiles.forEach((file, fi) => {
          jobs.push({ fieldName: `g${gi}_c${fi}`, file });
        });
      });
      const totalJobs = jobs.length;
      let completedJobs = 0;

      loadingText = `Uploading files (0/${totalJobs})...`;

      const UPLOAD_CONCURRENCY = 3;

      async function uploadOne(job: UploadJob): Promise<void> {
        const fd = new FormData();
        fd.append(job.fieldName, job.file);

        const resp = await fetch(
          `http://localhost:8080/api/upload/${sessionId}`,
          {
            method: 'POST',
            body: fd,
            signal: abortController?.signal,
          }
        );
        if (!resp.ok) {
          throw new Error(
            `Upload failed for ${job.fieldName}: ${resp.status} ${resp.statusText}`
          );
        }
        completedJobs++;
        loadingText = `Uploading files (${completedJobs}/${totalJobs})...`;
      }

      let nextJobIdx = 0;
      async function worker(): Promise<void> {
        while (true) {
          if (abortController?.signal.aborted) return;
          const myIdx = nextJobIdx++;
          if (myIdx >= jobs.length) return;
          await uploadOne(jobs[myIdx]);
        }
      }

      await Promise.all(
        Array.from({ length: Math.min(UPLOAD_CONCURRENCY, jobs.length) }, () => worker())
      );

      if (abortController.signal.aborted) {
        throw new DOMException('Upload cancelled', 'AbortError');
      }

      loadingText = 'Starting match...';
      const response = await fetch(
        `http://localhost:8080/api/match/${sessionId}`,
        {
          method: 'POST',
          signal: abortController.signal,
        }
      );

      if (!response.ok) {
        throw new Error(`Server error: ${response.status} ${response.statusText}`);
      }
      let gotComplete = false;

      for await (const frame of processMatchStream(response)) {
        if (abortController?.signal.aborted) break;

        switch (frame.type) {
          case 'chromosomeInfo': {
            if (chromosomeInfo.length === 0) {
              chromosomeInfo = frame.chromosomeInfo;
              hasChromosomeInfo = true;
              hasUploadedFiles = true;
              isLoading = false;
            }
            break;
          }

          case 'progress': {
            matchCount  = frame.progress.total_matches;
            recordCount = frame.progress.total_records;
            updateGenomeCountsFromBackend(frame.progress.per_genome_records);
            await new Promise(resolve => setTimeout(resolve, 0));
            break;
          }

          case 'complete': {
            matchCount          = frame.complete.total_matches;
            recordCount         = frame.complete.total_records;
            distinctContigCount = frame.complete.distinct_contig_count;
            updateGenomeCountsFromBackend(frame.complete.per_genome_records);
            gotComplete = true;
            break;
          }
        }
      }

      if (gotComplete) {
        streamComplete = true;
      }
    } catch (err) {
      if (err instanceof Error) {
        if (err.name === 'AbortError') {
          error = 'Upload cancelled';
        } else {
          error = err.message;
        }
      } else {
        error = 'Unknown error occurred';
      }

      if (sessionId) {
        void fetch(`http://localhost:8080/api/session/${sessionId}`, {
          method: 'DELETE',
        }).catch(() => { /* ignore cleanup failures */ });
        sessionId = null;
      }
    } finally {
      isLoading = false;
      isStreaming = false;
      abortController = null;
      loadingText = 'Initialising stream...';
    }
  }

  function resetUpload() {
    genomes = [];
    genomeRowCounts = [];
    fileToGenome = [];
    matches = [];
    chromosomeInfo = [];
    error = '';
    matchCount = 0;
    recordCount = 0;
    distinctContigCount = 0;
    hasUploadedFiles = false;
    hasChromosomeInfo = false;
    streamComplete = false;
    if (abortController) {
      abortController.abort();
      abortController = null;
    }
    if (sessionId) {
      void fetch(`http://localhost:8080/api/session/${sessionId}`, {
        method: 'DELETE',
      }).catch(() => { /* ignore cleanup failures */ });
      sessionId = null;
    }
  }

  function updateGenomeCountsFromBackend(perGenomeRecords: number[]) {
    genomeRowCounts = genomes.map((_, i) => perGenomeRecords[i] ?? 0);
    genomes = genomes.map((g, i) => ({
      ...g,
      rows: perGenomeRecords[i] ?? 0,
    }));
  }

  function cancelUpload() {
    if (abortController) {
      abortController.abort();
    }
  }
</script>

<main class="page">
  <div class="header">
    <h1>OGM Visualiser</h1>
    <div class="header-actions">
      <DarkModeToggle />
      {#if hasUploadedFiles && !isLoading}
        <button class="reset-button" on:click={resetUpload}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path d="M2 8a6 6 0 0 1 10.5-4M14 8a6 6 0 0 1-10.5 4" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
            <path d="M12.5 2v4h-4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
          Reupload Files
        </button>
      {/if}
    </div>
  </div>

  <TabNav bind:activeTab />
  {#if activeTab === 'visualization'}
    <div class="tab-content">
      {#if !hasUploadedFiles && !isLoading}
        <FileUpload
          on:upload={(e) => handleFileUpload(e.detail)}
          on:cancel={cancelUpload}
        />
      {/if}

      {#if error}
        <ErrorBanner {error} />
      {/if}

      {#if isLoading}
        <div class="loading-container">
          <LoadingSpinner />
          <p class="loading-text">{loadingText}</p>
        </div>
      {/if}

      {#if isStreaming && hasChromosomeInfo}
        <div class="streaming-banner">
          <div class="streaming-spinner"></div>
          <span>
            Processing matches...
            {matchCount.toLocaleString()} matches
            / {recordCount.toLocaleString()} records
            {#if streamComplete && distinctContigCount > 0}
              / {distinctContigCount.toLocaleString()} unique contigs
            {/if}
          </span>
          {#if !streamComplete}
            <button class="cancel-button-small" on:click={cancelUpload}>Cancel</button>
          {:else}
            <span class="complete-badge">✓ Complete</span>
          {/if}
        </div>
      {/if}

      {#if hasChromosomeInfo}
        <DisplayControls bind:showDuplicates/>
        <DonutVisualisation
          files={genomes}
          {fileToGenome}
          {matches}
          {chromosomeInfo}
          {showDuplicates}
          {sessionId}
          isQueryable={streamComplete && sessionId !== null}
          isStreaming={isStreaming && !streamComplete}
        />
      {/if}
    </div>

  {:else if activeTab === 'analysis'}
    <div class="tab-content">
      {#if streamComplete}
        {#if AreaAnalysis}
          <svelte:component
            this={AreaAnalysis}
            {matches}
            files={genomes}
            {fileToGenome}
            {chromosomeInfo}
            {sessionId}
            isQueryable={streamComplete && sessionId !== null}
          />
        {:else if loadingAreaAnalysis}
          <div class="placeholder-tab">
            <div class="streaming-placeholder">
              <LoadingSpinner />
              <p>Loading Area Analysis component...</p>
            </div>
          </div>
        {:else}
          <div class="placeholder-tab">
            <div class="streaming-placeholder">
              <LoadingSpinner />
              <p>Initializing Area Analysis...</p>
            </div>
          </div>
        {/if}
      {:else if isStreaming}
        <div class="placeholder-tab">
          <h2>Analysis Tab</h2>
          <div class="streaming-placeholder">
            <LoadingSpinner />
            <p>Loading data... {matchCount.toLocaleString()} matches so far</p>
          </div>
        </div>
      {:else}
        <div class="placeholder-tab">
          <h2>Analysis Tab</h2>
          <p class="data-status">No data loaded. Switch to Chromosome Flow tab to upload files.</p>
        </div>
      {/if}
    </div>
  {/if}
</main>

<style>
  /* ---------------------------------------------------------------------
     Theme variables
     ---------------------------------------------------------------------

     --------------------------------------------------------------------- */
  :global(:root) {
    --bg-primary: #ffffff;
    --bg-secondary: #f9fafb;
    --bg-hover: #f3f4f6;
    --bg-accent: #f0f9ff;
    --text-primary: #1f2937;
    --text-secondary: #6b7280;
    --text-tertiary: #9ca3af;
    --border-color: #e5e7eb;
    --border-color-dark: #d1d5db;
    --accent-primary: #3b82f6;
    --accent-hover: #2563eb;
    --accent-light: #dbeafe;
    --success: #10b981;
    --warning: #f59e0b;
    --error: #ef4444;
    --error-bg: #fef2f2;
    --error-border: #fecaca;
  }

  :global(.dark) {
    --bg-primary: #0f1419;
    --bg-secondary: #1a1f2e;
    --bg-hover: #242b3d;
    --bg-accent: #1e2b3f;
    --text-primary: #e8edf4;
    --text-secondary: #b1bfd0;
    --text-tertiary: #8f9db2;
    --border-color: #2d3748;
    --border-color-dark: rgb(50, 60, 80);
    --accent-primary: #5295e7;
    --accent-hover: #3b82f6;
    --accent-light: #1e3a5f;
    --success: #34d399;
    --warning: #fbbf24;
    --error: #f87171;
    --error-bg: #2d1f1f;
    --error-border: #4a2020;
  }

  :global(body) {
    background: var(--bg-primary);
    color: var(--text-primary);
    transition: background-color 0.2s, color 0.2s;
  }

  /* ---------------------------------------------------------------------
     Layout
     --------------------------------------------------------------------- */
  .page {
    padding: 2rem;
    max-width: 1400px;
    margin: 0 auto;
    background: var(--bg-primary);
  }

  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 2rem;
  }

  .header-actions {
    display: flex;
    gap: 1rem;
    align-items: center;
  }

  h1 { margin: 0;color: var(--text-primary);
  }

  /* ---------------------------------------------------------------------
     Reset / reupload button  outlined secondary
     --------------------------------------------------------------------- */
  .reset-button {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    padding: 0.625rem 1.25rem;
    background: var(--bg-primary);
    color: var(--accent-primary);
    border: 2px solid var(--accent-primary);
    border-radius: 0.5rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
    font-size: 0.875rem;
  }

  .reset-button:hover {
    background: var(--accent-primary);
    color: var(--bg-primary);
  }

  .tab-content {
    animation: fadeIn 0.2s ease-in;
  }

  @keyframes fadeIn {
    from { opacity: 0; transform: translateY(-4px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  /* ---------------------------------------------------------------------
     Loading state (pre-stream)
     --------------------------------------------------------------------- */
  .loading-container {
    display: flex;
    gap: 1rem;
    flex-direction: column;
    align-items: center;
    padding: 4rem;
    background: var(--bg-secondary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
  }

  .loading-text {
    color: var(--text-secondary);
    font-weight: 500;
  }

  /* ---------------------------------------------------------------------
     Streaming banner (during active ingestion)
     --------------------------------------------------------------------- */
  .streaming-banner {
    display: flex;
    gap: 1rem;
    align-items: center;
    padding: 1rem 1.5rem;
    background: var(--accent-light);
    border-radius: 0.5rem;
    border: 1px solid var(--accent-primary);
    margin-bottom: 1.5rem;
    animation: slideDown 0.3s ease-out;
  }

  @keyframes slideDown {
    from { opacity: 0; transform: translateY(-10px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  .streaming-spinner {
    width: 20px;
    height: 20px;
    border: 3px solid var(--accent-primary);
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .cancel-button-small {
    margin-left: auto;
    padding: 0.375rem 1rem;
    background: var(--error);
    color: white;
    border: none;
    border-radius: 0.375rem;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.2s, filter 0.2s, transform 0.1s;
  }

  .cancel-button-small:hover {
    background: #dc2626;
    filter: brightness(1.05);
  }

  :global(.dark) .cancel-button-small:hover {
    background: #ef4444;
    filter: brightness(1.1);
  }

  .cancel-button-small:active {
    transform: translateY(1px);
  }

  .complete-badge {
    margin-left: auto;
    padding: 0.25rem 0.75rem;
    background: var(--success);
    color: white;
    border-radius: 0.25rem;
    font-size: 0.875rem;
    font-weight: 600;
  }

  /* ---------------------------------------------------------------------
     Placeholder (empty analysis tab)
     --------------------------------------------------------------------- */
  .placeholder-tab {
    text-align: center;
    padding: 4rem;
    background: var(--bg-secondary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
  }

  .placeholder-tab h2 {
    color: var(--text-primary);
    margin-bottom: 1rem;
  }

  .placeholder-tab p {
    color: var(--text-secondary);
    margin-bottom: 0.5rem;
  }

  .streaming-placeholder {
    display: flex;
    gap: 1rem;
    flex-direction: column;
    align-items: center;
    margin-top: 2rem;
  }

  .data-status {
    margin-top: 2rem;
    padding: 1rem;
    background: var(--bg-accent);
    border-radius: 0.375rem;
    font-weight: 500;
    color: var(--text-primary);
  }
</style>