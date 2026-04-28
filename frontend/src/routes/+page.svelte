<script lang="ts">
  import { onMount } from 'svelte';
  import {
    processMatchStream,
    type ChromosomeInfo,
  } from '$lib/bincodeDecoder';
  import { darkMode } from '$lib/darkModeStore';
  import { donutFltState } from '$lib/filterStateStore';
  import { resetSearchStore } from '$lib/searchStore';
  import FileUpload from './FileUpload.svelte';
  import ErrorBanner from './ErrorBanner.svelte';
  import LoadingSpinner from './LoadingSpinner.svelte';
  import TabNav from './TabNav.svelte';
  import DarkModeToggle from './DarkModeToggle.svelte';
  import DonutVisualisation from './DonutVisualisation.svelte';
  import DisplayControls from './DisplayControls.svelte';
  let AreaAnalysis: any = null;
  let loadingArea = false;

  interface GenData {
    name: string;
    fileCnt: number;
    rows: number;
    color: string;
  }

  const GEN_COLORS = ['#3b82f6', '#10b981', '#f59e0b'];
  const API = 'http://localhost:8080';
  const UP_CONCURRENCY = 3;

  let genomes: GenData[] = [];
  let fileToGen: number[] = [];
  let genRowCnts: number[] = [];
  let chrInfo: ChromosomeInfo[][] = [];

  let isLoading = false;
  let loadingText = 'Initialising stream...';

  let sessId: string | null = null;
  let isStreaming = false;
  let error = '';

  let mtcCnt = 0;
  let recCnt = 0;
  let distSeqCnt = 0;
  let abortCtrl: AbortController | null = null;
  let showDups = false;

  let activeTab: 'viz' | 'anlys' = 'viz';
  let hasUploadedFiles = false;
  let hasChrInfo = false;
  let streamComplete = false;

  onMount(() => {
    document.documentElement.classList.toggle('dark', $darkMode);
  });

  async function loadArea() {
    if (AreaAnalysis || loadingArea) return;
    loadingArea = true;
    try {
      const mod = await import('./AreaAnalysis.svelte');
      AreaAnalysis = mod.default;
    } catch (err) {
      console.error('Failed to load AreaAnalysis:', err);
    } finally {
      loadingArea = false;
    }
  }

  $: if (activeTab === 'anlys' && streamComplete) loadArea();

  function deleteSession(id: string) {
    void fetch(`${API}/api/session/${id}`, { method: 'DELETE' }).catch(() => {});
  }

  async function onFileUpload(genGroups: { seqFiles: File[]; refineFinalFile: File; dirName: string }[]) {
    if (!genGroups || genGroups.length < 2) return;

    if (abortCtrl) abortCtrl.abort();
    if (sessId) { deleteSession(sessId); sessId = null; }

    abortCtrl = new AbortController();
    isLoading = true;
    isStreaming = true;
    streamComplete = false;
    error = '';
    chrInfo = [];
    mtcCnt = 0;
    recCnt = 0;
    distSeqCnt = 0;
    hasUploadedFiles = false;
    hasChrInfo = false;

    donutFltState.reset();
    resetSearchStore();

    genomes = genGroups.map((g, i) => ({
      name: g.dirName,
      fileCnt: g.seqFiles.length,
      rows: 0,
      color: GEN_COLORS[i],
    }));

    genRowCnts = new Array(genomes.length).fill(0);

    fileToGen = genGroups.flatMap((g, gi) =>
      Array.from({ length: g.seqFiles.length }, () => gi),
    );

    try {
      loadingText = 'Creating session...';
      const sessResp = await fetch(`${API}/api/session`, {
        method: 'POST',
        signal: abortCtrl.signal,
      });
      if (!sessResp.ok) {
        throw new Error(`Session create failed: ${sessResp.status} ${sessResp.statusText}`);
      }
      const { session_id } = await sessResp.json() as { session_id: string };
      sessId = session_id;

      type UpJob = { fieldName: string; file: File };
      const jobs: UpJob[] = [];
      genGroups.forEach((group, gi) => {
        jobs.push({ fieldName: `g${gi}_r`, file: group.refineFinalFile });
        group.seqFiles.forEach((file, fi) => {
          jobs.push({ fieldName: `g${gi}_s${fi}`, file });
        });
      });
      const totJobs = jobs.length;
      let doneJobs = 0;

      loadingText = `Uploading files (0/${totJobs})...`;

      async function upOne(job: UpJob): Promise<void> {
        const fd = new FormData();
        fd.append(job.fieldName, job.file);

        const resp = await fetch(`${API}/api/upload/${sessId}`, {
          method: 'POST',
          body: fd,
          signal: abortCtrl?.signal,
        });
        if (!resp.ok) {
          throw new Error(
            `Upload failed for ${job.fieldName}: ${resp.status} ${resp.statusText}`,
          );
        }
        doneJobs++;
        loadingText = `Uploading files (${doneJobs}/${totJobs})...`;
      }

      let nextJobIdx = 0;
      async function worker(): Promise<void> {
        while (true) {
          if (abortCtrl?.signal.aborted) return;
          const myIdx = nextJobIdx++;
          if (myIdx >= jobs.length) return;
          await upOne(jobs[myIdx]);
        }
      }

      await Promise.all(
        Array.from({ length: Math.min(UP_CONCURRENCY, jobs.length) }, () => worker()),
      );

      if (abortCtrl.signal.aborted) {
        throw new DOMException('Upload cancelled', 'AbortError');
      }

      loadingText = 'Starting match...';
      const resp = await fetch(`${API}/api/match/${sessId}`, {
        method: 'POST',
        signal: abortCtrl.signal,
      });

      if (!resp.ok) {
        throw new Error(`Server error: ${resp.status} ${resp.statusText}`);
      }
      let gotComplete = false;

      for await (const frame of processMatchStream(resp)) {
        if (abortCtrl?.signal.aborted) break;

        switch (frame.type) {
          case 'chromosomeInfo':
            if (chrInfo.length === 0) {
              chrInfo = frame.chromosomeInfo;
              hasChrInfo = true;
              hasUploadedFiles = true;
              isLoading = false;
            }
            break;

          case 'progress':
            mtcCnt = frame.progress.total_matches;
            recCnt = frame.progress.total_records;
            updateGenCntsFromBE(frame.progress.per_genome_records);
            await new Promise(resolve => setTimeout(resolve, 0));
            break;

          case 'complete':
            mtcCnt = frame.complete.total_matches;
            recCnt = frame.complete.total_records;
            distSeqCnt = frame.complete.distinct_sequence_count;
            updateGenCntsFromBE(frame.complete.per_genome_records);
            gotComplete = true;
            break;
        }
      }

      if (gotComplete) streamComplete = true;
    } catch (err) {
      if (err instanceof Error) {
        error = err.name === 'AbortError' ? 'Upload cancelled' : err.message;
      } else {
        error = 'Unknown error occurred';
      }

      if (sessId) { deleteSession(sessId); sessId = null; }
    } finally {
      isLoading = false;
      isStreaming = false;
      abortCtrl = null;
      loadingText = 'Initialising stream...';
    }
  }

  function resetUpload() {
    genomes = [];
    genRowCnts = [];
    fileToGen = [];
    chrInfo = [];
    error = '';
    mtcCnt = 0;
    recCnt = 0;
    distSeqCnt = 0;
    hasUploadedFiles = false;
    hasChrInfo = false;
    streamComplete = false;
    if (abortCtrl) { abortCtrl.abort(); abortCtrl = null; }
    if (sessId) { deleteSession(sessId); sessId = null; }
  }

  function updateGenCntsFromBE(perGenRecs: number[]) {
    genRowCnts = genomes.map((_, i) => perGenRecs[i] ?? 0);
    genomes = genomes.map((g, i) => ({ ...g, rows: perGenRecs[i] ?? 0 }));
  }

  function cancelUpload() {
    if (abortCtrl) abortCtrl.abort();
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
  {#if activeTab === 'viz'}
    <div class="tab-content">
      {#if !hasUploadedFiles && !isLoading}
        <FileUpload
          on:upload={(e) => onFileUpload(e.detail)}
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

      {#if isStreaming && hasChrInfo}
        <div class="streaming-banner">
          <div class="streaming-spinner"></div>
          <span>
            Processing matches...
            {mtcCnt.toLocaleString()} matches
            / {recCnt.toLocaleString()} records
            {#if streamComplete && distSeqCnt > 0}
              / {distSeqCnt.toLocaleString()} unique sequences
            {/if}
          </span>
          {#if !streamComplete}
            <button class="cancel-button-small" on:click={cancelUpload}>Cancel</button>
          {:else}
            <span class="complete-badge">✓ Complete</span>
          {/if}
        </div>
      {/if}

      {#if hasChrInfo}
        <DisplayControls bind:showDups/>
        <DonutVisualisation
          files={genomes}
          {fileToGen}
          {chrInfo}
          {showDups}
          {sessId}
          isQueryable={streamComplete && sessId !== null}
          isStreaming={isStreaming && !streamComplete}
        />
      {/if}
    </div>

  {:else if activeTab === 'anlys'}
    <div class="tab-content">
      {#if streamComplete}
        {#if AreaAnalysis}
          <svelte:component
            this={AreaAnalysis}
            files={genomes}
            {fileToGen}
            {chrInfo}
            {sessId}
            isQueryable={streamComplete && sessId !== null}
          />
        {:else if loadingArea}
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
            <p>Loading data... {mtcCnt.toLocaleString()} matches so far</p>
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

  /* Layout */
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