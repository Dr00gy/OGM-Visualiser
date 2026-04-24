<script lang="ts">
  /**
   * AreaAnalysis
   * -----------------------------------------------------------------------
   * The "Analytic Browser" tab: a window-at-a-time view of contig alignments
   * on a chosen chromosome, rendered as stacked horizontal bars where each
   * bar is one alignment. The user can:
   *
   *   - pick which GENOMES to include (not files — selections collapse to
   *     the genome level via fileToGenome[]),
   *   - pick a chromosome (1..24),
   *   - page through fixed-size windows of that chromosome,
   *   - search for a specific query contig ID (jumps to windows containing it),
   *   - expand the "Chromosome Overview" panel to see hit density across all
   *     chromosomes at once,
   *   - expand the "Window Sequence Comparison" panel to see which contigs
   *     are shared / unique per genome inside the current window.
   *
   * Performance considerations
   * --------------------------
   * This component is large because it handles potentially millions of
   * records, so it leans heavily on caching layers:
   *
   *   LAYER 1 — colorCache:
   *       Memoises HSL colour generation per contigId.
   *   LAYER 2 — cachedContigBars:
   *       The computed bar geometry (x/width/key) for the current window.
   *       Invalidated when window bounds or the filtered record list change.
   *   LAYER 3 — cachedStackedContigs:
   *       The stacked-track layout. Same invalidation as LAYER 2.
   *
   * Plus an IntersectionObserver gate: the whole component does no work
   * until it scrolls into view (isVisible), because the user may switch
   * tabs / scroll away before we'd want to pay for reactive computations.
   *
   * Filter-state store vs local vars
   * --------------------------------
   * `areaAnalysisFilterState` persists across reloads via localStorage.
   * We keep local mirrors (selectedGenomes, windowSize, etc.) for template
   * ergonomics and push changes back through `areaAnalysisFilterState.update`.
   * `searchStore` likewise persists the search query across tab switches.
   *
   * IMPORTANT: Originally this file imported from relative paths
   * (`../lib/...`) which desynced from the rest of the codebase that uses
   * `$lib/...`. Mixed aliases caused `FileData` to resolve to the wrong
   * module under some build configs. Both imports below have been
   * normalised to `$lib/...`.
   */

  import { onMount, onDestroy } from 'svelte';
  import type { BackendMatch, ChromosomeInfo } from '$lib/bincodeDecoder';
  import type { FileData } from '$lib/types';
  import { searchStore } from '$lib/searchStore';
  import { areaAnalysisFilterState } from '$lib/filterStateStore';
  import {
    fetchChromosomeRecords,
    fetchContigLocations,
    type WireAreaRecord,
    type ChromosomeRecordsResponse,
    type ContigLocation,
  } from '$lib/queryClient';

  /**
   * Component props — all data comes in from +page.svelte, the component
   * does not fetch its own.
   *
   * Phase 1b: `matches` is now deprecated but kept as a no-op prop for
   * backwards compatibility with callers. Chromosome data is fetched
   * from the backend via `sessionId` below. When `isQueryable` is false
   * the component sits dormant (shows empty state); when true, visible,
   * and the user has picked genomes+chromosome, a fetch happens.
   */
  export let matches: BackendMatch[] = [];
  /** One entry per genome (2–3). name/color come from _page.svelte genomes array. */
  export let files: FileData[] = [];
  /**
   * Maps flat file_index (record.file_index from backend) → genome index.
   * Used to translate records to their genome before filtering/display.
   */
  export let fileToGenome: number[] = [];
  /** Per-genome chromosome info from backend (ref_contig_id + ref_len per chromosome). */
  export let chromosomeInfo: ChromosomeInfo[][] = [];

  /**
   * Phase 1b: session id for backend queries. `null` means no active
   * session (pre-upload or post-reset).
   */
  export let sessionId: string | null = null;

  /**
   * Phase 1b: true once the match phase has completed and the session
   * is ready to answer query endpoints. Fetches are gated on this.
   */
  export let isQueryable: boolean = false;

  /**
   * Lazy-loading state: the IntersectionObserver flips `isVisible` to true
   * once the component scrolls into view, and `isInitialized` is a latch
   * to guarantee we only wire up subscriptions / observer logic once.
   */
  let isVisible = false;
  let containerElement: HTMLElement;
  let isInitialized = false;

  /**
   * Genome selection state (replaces per-file selectedFiles).
   * Holds genome indices (0, 1, 2) that are currently active.
   * Stored in areaAnalysisFilterState.selectedFiles for persistence —
   * the store field name predates the rename from "files" to "genomes".
   */
  let selectedGenomes: number[] = [];
  /** Which chromosome (1..24) is currently being browsed. */
  let selectedChromosome = 1;
  /** Window size in bp. 100 kb is a reasonable default for most use cases. */
  let windowSize = 100000;
  /** Zero-based window index within the chromosome. */
  let currentWindowIndex = 0;
  /** The contig record currently hovered by the mouse (tooltip source). */
  let hoveredContig: any = null;

  // -------------------------------------------------------------------------
  // Phase 1b: chromosome-record cache + async fetch machinery
  // -------------------------------------------------------------------------
  //
  // The old code kept `chromosomeRecords` as a synchronous derivation
  // from `matches: BackendMatch[]`. In Phase 1b, `matches` is empty —
  // records live server-side and are fetched per (genomes, chromosome)
  // on demand.
  //
  // Cache key: `"{sorted genome indices}|{chromosome}"`. When the user
  // switches chromosome, we check the cache first; only cold keys hit
  // the server. When `selectedGenomes` changes (genome selection set
  // changes) the whole cache is invalidated because the key semantics
  // have changed — we never want to accidentally serve a response
  // computed against a different genome set.
  //
  // The cache is NOT persisted. Each new upload gets a fresh session id
  // and therefore a fresh cache (the reactive block below clears on
  // sessionId change too).

  /**
   * Per-chromosome response cache. Map<cacheKey, response>.
   *
   * Using a plain Map rather than a Svelte store because the cache
   * updates don't need to be reactive — the `chromosomeRecords` array
   * that the template reads is a separate reactive variable that gets
   * assigned from the cache.
   */
  let chromosomeRecordCache = new Map<string, ChromosomeRecordsResponse>();

  /**
   * Current in-flight request, so we can abort when the user rapidly
   * changes selections. At most one fetch is in flight at a time.
   */
  let chromosomeRecordsAbort: AbortController | null = null;

  /** True while a chromosome-records fetch is in progress. */
  let chromosomeRecordsLoading = false;

  /**
   * Tracks the cache key whose response is currently displayed, so
   * stale responses (ones that arrived after a later fetch kicked off)
   * don't overwrite newer ones.
   */
  let chromosomeRecordsActiveKey: string | null = null;

  /**
   * The chromosome records currently driving the UI. Populated from
   * either the cache (sync) or from a fresh fetch (async). Replaces
   * the old `$: chromosomeRecords = getRecordsForChromosome(matches, ...)`.
   *
   * Shape is the server's `WireAreaRecord` with an added `genome_index`
   * field (the server already pre-resolves it), which mirrors the
   * shape the old template expected.
   */
  let chromosomeRecords: WireAreaRecord[] = [];

  /**
   * Chromosome reference length reported by the server. Previously read
   * off the first record's `ref_len`; we now carry the response-level
   * value so empty record sets still know the chromosome length.
   */
  let chromosomeRefLenFetched: number = 0;

  /** Cache key helper. */
  function cacheKey(genomes: number[], chr: number): string {
    // Sort so `[0, 1]` and `[1, 0]` produce the same key.
    return `${[...genomes].sort((a, b) => a - b).join(',')}|${chr}`;
  }

  /**
   * Load chromosome records for the current selection, using the cache
   * when available. Writes `chromosomeRecords` and
   * `chromosomeRefLenFetched` on success.
   *
   * No-op if session isn't queryable, component isn't visible, or the
   * user hasn't picked any genomes (empty selection → empty records).
   */
  async function reloadChromosomeRecords() {
    // Clear the current displayed set if preconditions aren't met, so
    // the UI doesn't show stale data for a different selection.
    if (!sessionId || !isQueryable || !isVisible) {
      chromosomeRecords = [];
      chromosomeRefLenFetched = 0;
      chromosomeRecordsActiveKey = null;
      return;
    }
    if (selectedGenomes.length === 0) {
      chromosomeRecords = [];
      chromosomeRefLenFetched = 0;
      chromosomeRecordsActiveKey = null;
      return;
    }

    const key = cacheKey(selectedGenomes, selectedChromosome);

    // Cache hit → serve synchronously. Also clears caches on the bar-
    // stacking layer so geometry gets regenerated for the new dataset.
    const cached = chromosomeRecordCache.get(key);
    if (cached) {
      chromosomeRecords = cached.records;
      chromosomeRefLenFetched = cached.chromosome_ref_len;
      chromosomeRecordsActiveKey = key;
      clearCaches();
      return;
    }

    // Cold — kick off a fetch. Abort any prior one first.
    if (chromosomeRecordsAbort) {
      chromosomeRecordsAbort.abort();
    }
    chromosomeRecordsAbort = new AbortController();
    const signal = chromosomeRecordsAbort.signal;

    // Delayed loading flag — only flip true if the fetch takes long.
    // Prevents a loading-chip flicker on fast responses.
    const chipTimer = setTimeout(() => { chromosomeRecordsLoading = true; }, 200);

    try {
      const resp = await fetchChromosomeRecords(sessionId, {
        genomes: selectedGenomes,
        chr: selectedChromosome,
        signal,
      });
      // `undefined` = aborted; keep whatever's currently showing.
      if (resp === undefined) return;

      // Check the key is still relevant — if the user changed
      // selection while we were waiting, a newer fetch is in flight
      // and we shouldn't clobber its result.
      const currentKey = cacheKey(selectedGenomes, selectedChromosome);
      if (key !== currentKey) return;

      chromosomeRecordCache.set(key, resp);
      chromosomeRecords = resp.records;
      chromosomeRefLenFetched = resp.chromosome_ref_len;
      chromosomeRecordsActiveKey = key;
      clearCaches();
    } catch (err) {
      console.error('Failed to fetch chromosome records:', err);
      chromosomeRecords = [];
      chromosomeRefLenFetched = 0;
    } finally {
      clearTimeout(chipTimer);
      chromosomeRecordsLoading = false;
    }
  }

  /**
   * Fetch trigger. Runs whenever (sessionId, isQueryable, isVisible,
   * selectedGenomes, selectedChromosome) changes.
   *
   * A deliberate choice: we do NOT debounce this fetch. Chromosome
   * switches are user-driven single clicks that feel slow if
   * debounced; the AbortController + stale-key check keeps the
   * response handling safe.
   */
  $: {
    void sessionId;
    void isQueryable;
    void isVisible;
    void selectedGenomes;
    void selectedChromosome;
    reloadChromosomeRecords();
  }

  /**
   * Cache invalidation on genome-selection change.
   *
   * Previously-cached entries were computed for a different genome set,
   * so serving them would mix data. When `selectedGenomes` mutates (new
   * array identity), we drop everything.
   */
  let lastGenomesKey = '';
  $: {
    const gk = [...selectedGenomes].sort((a, b) => a - b).join(',');
    if (gk !== lastGenomesKey) {
      lastGenomesKey = gk;
      chromosomeRecordCache.clear();
      chromosomeRecordsActiveKey = null;
    }
  }

  /** Cache invalidation on session change. Gated so it only fires
   *  when `sessionId` actually changes, not on every reactive tick. */
  let lastSessionId: string | null = null;
  $: {
    if (sessionId !== lastSessionId) {
      lastSessionId = sessionId;
      chromosomeRecordCache.clear();
      chromosomeRecordsActiveKey = null;
    }
  }

  // -------------------------------------------------------------------------
  // Contig-location cache for the overview search
  // -------------------------------------------------------------------------
  //
  // Separate from chromosome-records cache. Key: qry_contig_id.
  // Populated by `fetchContigLocations` when the user searches for a
  // specific contig AND the overview panel is open (or the search
  // feature needs cross-chromosome hit info).

  let contigLocationsCache = new Map<number, ContigLocation[]>();
  let contigLocationsAbort: AbortController | null = null;

  /**
   * Locations for the currently-searched contig, or `null` when there's
   * no active contig search. Used by the overview panel and the
   * "windows containing contig" navigation.
   */
  let activeContigLocations: ContigLocation[] | null = null;

  /** Fetch + cache locations for a given contig id. */
  async function loadContigLocations(contigId: number) {
    if (!sessionId || !isQueryable) {
      activeContigLocations = null;
      return;
    }
    const cached = contigLocationsCache.get(contigId);
    if (cached) {
      activeContigLocations = cached;
      return;
    }
    if (contigLocationsAbort) contigLocationsAbort.abort();
    contigLocationsAbort = new AbortController();
    try {
      const resp = await fetchContigLocations(sessionId, {
        qry: contigId,
        genomes: selectedGenomes,
        signal: contigLocationsAbort.signal,
      });
      if (!resp) return; // aborted
      contigLocationsCache.set(contigId, resp.locations);

      // Install the result if this contig is still what the user has
      // submitted. We compare against `submittedSearchQuery` directly
      // rather than going through `isSearching` because `isSearching`
      // can be stale mid-tick when the filter store is in the middle
      // of propagating.
      const currentRaw = submittedSearchQuery.trim();
      if (currentRaw !== '') {
        const current = parseInt(currentRaw, 10);
        if (!Number.isNaN(current) && current === contigId) {
          activeContigLocations = resp.locations;
        }
      }
    } catch (err) {
      console.error('Failed to fetch contig locations:', err);
      activeContigLocations = null;
    }
  }

  /**
   * React to search state: when the user submits a contig search, load
   * its cross-chromosome locations. Clear when search is cleared.
   */
  $: if (isSearching && submittedSearchQuery) {
    const id = parseInt(submittedSearchQuery);
    if (!Number.isNaN(id)) loadContigLocations(id);
  } else {
    activeContigLocations = null;
  }

  /**
   * Invalidate the contig-locations cache on genome change.
   *
   * Uses the same `lastGenomesKey` guard as the chromosome-records
   * cache so this block only fires when `selectedGenomes` actually
   * changes, not on every spurious reactive re-evaluation.
   *
   * If a search is active when the genomes change, we also re-trigger
   * the load with the new filter so dots stay in sync.
   */
  let lastLocationsGenomesKey = '';
  $: {
    const gk = [...selectedGenomes].sort((a, b) => a - b).join(',');
    if (gk !== lastLocationsGenomesKey) {
      lastLocationsGenomesKey = gk;
      contigLocationsCache.clear();
      if (contigLocationsAbort) {
        contigLocationsAbort.abort();
        contigLocationsAbort = null;
      }
      // If a search is active, fetch again for the new genome filter.
      if (isSearching && submittedSearchQuery) {
        const id = parseInt(submittedSearchQuery);
        if (!Number.isNaN(id)) loadContigLocations(id);
      }
    }
  }

  /**
   * Invalidate contig-locations cache on session change. Guarded so
   * it only fires when sessionId actually changes, not on every
   * reactive tick that happens to mention sessionId.
   */
  let lastLocationsSessionId: string | null = null;
  $: {
    if (sessionId !== lastLocationsSessionId) {
      lastLocationsSessionId = sessionId;
      contigLocationsCache.clear();
      if (contigLocationsAbort) {
        contigLocationsAbort.abort();
        contigLocationsAbort = null;
      }
    }
  }


  /**
   * Search state (live vs submitted).
   *
   * `searchQuery` is bound to the input field — it changes on every
   * keystroke. `submittedSearchQuery` is only updated when the user
   * presses Enter; all filtering downstream uses the submitted value
   * so we don't rebuild results per-character.
   */
  let searchQuery = '';
  let submittedSearchQuery = '';
  
  /**
   * When searching by contig ID, this holds the window indices that
   * contain at least one hit for that contig. The user's prev/next
   * buttons then navigate through this subset instead of all windows.
   */
  let filteredWindows: number[] = [];
  let isSearching = false;

  /**
   * On initial mount, if there's a persisted search query we want to
   * re-execute it — but only after subscriptions are wired up. This
   * flag signals the subscription to do that once.
   */
  let shouldReRunSearch = false;

  /**
   * Chromosome Overview Panel state.
   * Expanded on-demand because its computation is non-trivial.
   */
  let overviewPanelOpen = false;

  /** Represents one dot (a cluster of search hits) on a chromosome line in the overview. */
  interface OverviewDot {
    /** Fractional position along the chromosome line (0..1). */
    xFraction: number;
    /** Estimated window index the user would land on if they click this dot. */
    estimatedWindow: number;
  }

  /** One chromosome's horizontal line in the overview panel. */
  interface ChromosomeLine {
    chrId: number;
    refLen: number;
    /** 12 tick markers (start + 10 intermediate + end) — positions in bp. */
    markers: number[];
    /** Dots aggregated from the search hits that fall on this chromosome. */
    dots: OverviewDot[];
  }

  /**
   * Build chromosome overview lines for a given genome.
   *
   * For each chromosome:
   *   - Generate 12 evenly-spaced markers along its length (purely visual
   *     scale ticks — they are NOT involved in locating dots).
   *   - If there's an active search contig, compute the exact window
   *     index for every matching hit and drop a dot at that window's
   *     position. Dots that share a window are deduplicated.
   *
   * Window-coordinate alignment
   * ---------------------------
   * The rest of this component tiles windows starting at
   * `chromosomeRange.min` (the lowest `ref_start_pos` across records on
   * the chromosome — see `windowStart` in the reactive chain and
   * `findWindowsWithContig`). `rangeMin` below mirrors that exact
   * calculation so `estimatedWindow` is the SAME window index the
   * viewport would tile to.
   *
   * Why not bin hits into the 12 markers?
   * -------------------------------------
   * An earlier version placed dots at the midpoint of whichever
   * marker-interval a hit fell into. For a 200 Mb chromosome with 12
   * markers that's ~18 Mb per interval, i.e. ~180 windows at 100 kb —
   * so clicks could land up to 90 windows away from the real hit. We
   * now compute the window index directly from the hit's bp midpoint.
   */
  function buildChromosomeLines(
    genomeIdx: number,
    chrInfoForGenome: ChromosomeInfo[],
    contigId: number | null,
    _selectedGenomes: number[],
  ): ChromosomeLine[] {
    // Sort chromosomes by ref_contig_id so overview lines always appear in
    // numerical order regardless of how the backend delivered them.
    const sorted = [...chrInfoForGenome].sort((a, b) => a.ref_contig_id - b.ref_contig_id);

    const genomeIsSelected = _selectedGenomes.includes(genomeIdx);

    // Phase 1b: dot placement sources are different per chromosome:
    //   - For the currently-displayed chromosome, use `chromosomeRange.min`
    //     from the main reactive chain (already computed from the fetched
    //     records) so dots line up with the viewport precisely.
    //   - For OTHER chromosomes, we don't have record-level data for them
    //     in memory (that's the point of Phase 1b — one chromosome at a
    //     time). We fall back to `rangeMin = 0`, which means dot positions
    //     may be off by up to one window for chromosomes with records
    //     starting well above 0 bp. The `navigateFromDot` handler snaps
    //     to the nearest actual hit window on click, so this is a visual
    //     quirk, not a navigational bug.
    //
    // Hit dots come from `activeContigLocations` — a one-shot fetch that
    // lists every position of the searched contig across all genomes.
    const locs: ContigLocation[] = activeContigLocations ?? [];

    return sorted.map(chr => {
      const refLen = chr.ref_len;
      const numMarkers = 12;
      const markers: number[] = [];
      for (let i = 0; i < numMarkers; i++) {
        markers.push(Math.round((refLen * i) / (numMarkers - 1)));
      }

      let dots: OverviewDot[] = [];

      if (contigId !== null) {
        // Compute rangeMin: exact for the currently-displayed chromosome
        // (using the already-fetched chromosomeRecords), zero for others.
        let rangeMin = 0;
        if (genomeIsSelected && chr.ref_contig_id === selectedChromosome) {
          // Use the value the main reactive chain uses so dots align with
          // the viewport exactly on the active chromosome.
          rangeMin = Math.floor(chromosomeRange.min);
        }

        // The bp span that windows actually cover on this chromosome.
        // Must match `totalWindows * windowSize` in the reactive chain so
        // the dot's x position maps correctly onto the windowed viewport.
        const totalWindowsForChr = Math.max(
          1,
          Math.ceil(Math.max(0, refLen - rangeMin) / windowSize),
        );
        const windowedSpan = totalWindowsForChr * windowSize;

        // Walk the contig's locations. Each alignment may SPAN multiple
        // windows; emit one dot per window in the span so the overview
        // matches the "N windows" count the header shows — the main
        // viewport's `filteredWindows` uses the same span-inclusive
        // semantics (see `findWindowsWithContig`).
        //
        // Multiple alignments landing in the same window collapse via
        // the Set — we don't want to render two dots in identical
        // positions on top of each other.
        const hitWindows = new Set<number>();
        for (const loc of locs) {
          if (loc.genome_index !== genomeIdx) continue;
          if (loc.ref_contig_id !== chr.ref_contig_id) continue;

          // Start and end windows of this alignment's span, both inclusive.
          const relStart = Math.max(0, loc.ref_start_pos - rangeMin);
          const relEnd   = Math.max(0, loc.ref_end_pos   - rangeMin);
          const startWindow = Math.min(
            totalWindowsForChr - 1,
            Math.floor(relStart / windowSize),
          );
          const endWindow = Math.min(
            totalWindowsForChr - 1,
            Math.floor(relEnd / windowSize),
          );
          for (let w = startWindow; w <= endWindow; w++) {
            if (w >= 0) hitWindows.add(w);
          }
        }

        // Turn each distinct window into a dot. xFraction is computed from
        // the window's bp offset so the dot sits over its window.
        for (const windowIdx of hitWindows) {
          const windowCenterRel = (windowIdx + 0.5) * windowSize;
          const xFraction = windowedSpan > 0
            ? Math.min(1, windowCenterRel / windowedSpan)
            : 0;

          dots.push({ xFraction, estimatedWindow: windowIdx });
        }

        // Left-to-right visual order.
        dots.sort((a, b) => a.xFraction - b.xFraction);
      }

      return {
        chrId: chr.ref_contig_id,
        refLen,
        markers,
        dots,
      };
    });
  }

  /**
   * Reactive: per-genome chromosome overview lines.
   *
   * Only computed when the panel is open — we don't want to pay for this
   * on every chromosome change if the user isn't looking at the overview.
   *
   * The function arguments are explicitly listed so Svelte's reactive
   * dependency tracker sees them; the body uses the outer variables (which
   * is fine — the explicit args just pin the dependency graph).
   */
  /**
   * Reactive: per-genome chromosome overview lines.
   *
   * Only computed when the panel is open — we don't want to pay for this
   * on every chromosome change if the user isn't looking at the overview.
   *
   * Phase 1b: the reactive deps include `activeContigLocations` so the
   * panel re-renders when the search-contig fetch resolves.
   */
  $: overviewData = overviewPanelOpen
    ? buildOverviewData(isSearching, submittedSearchQuery, activeContigLocations, chromosomeInfo, files, fileToGenome, windowSize, selectedGenomes)
    : [];

  /**
   * Builds the full overview data structure — one entry per genome with
   * its name, colour, and per-chromosome lines.
   *
   * Phase 1b: `activeContigLocations` replaces the old `matches` walk.
   * It's listed as an explicit parameter so Svelte's reactive dependency
   * tracker pins it as a dep even though `buildChromosomeLines` reads it
   * from the outer scope.
   */
  function buildOverviewData(
    _isSearching: boolean,
    _submittedSearchQuery: string,
    _activeContigLocations: ContigLocation[] | null,
    _chromosomeInfo: ChromosomeInfo[][],
    _files: FileData[],
    _fileToGenome: number[],
    _windowSize: number,
    _selectedGenomes: number[],
  ): { genomeName: string; genomeColor: string; lines: ChromosomeLine[] }[] {
    // Parse the active search contig ID once; null if no search or malformed.
    const searchContigId = (isSearching && submittedSearchQuery)
      ? parseInt(submittedSearchQuery)
      : null;
    const parsedContigId = (searchContigId !== null && !isNaN(searchContigId)) ? searchContigId : null;

    return chromosomeInfo.map((chrInfo, gi) => ({
      genomeName: files[gi]?.name ?? `Genome ${gi}`,
      genomeColor: files[gi]?.color ?? '#888',
      lines: buildChromosomeLines(gi, chrInfo, parsedContigId, _selectedGenomes),
    }));
  }

  /**
   * Navigate to a specific window from an overview-dot click.
   *
   * Switches to the chromosome that owns the dot AND jumps to the
   * estimated window — both in one atomic update so the user doesn't
   * see a stale chromosome/window combo flash by.
   *
   * Two subtle concerns handled here:
   *
   *   1. `filteredWindows` is per-chromosome. When we hop to a new
   *      chromosome while searching, the old hit set is meaningless —
   *      its window indices refer to the previous chromosome's tiling.
   *      If we don't rebuild it, `effectiveCurrentWindowIndex` falls
   *      back to `|| 1` (because the new raw index isn't in the old
   *      set), `canGoNext/Prev` walk an unrelated subset, and the
   *      "window X / N" readout is a lie. Rebuild here.
   *
   *   2. Dot `estimatedWindow` is computed in `buildChromosomeLines`
   *      against a `rangeMin` derived from the SINGLE genome whose row
   *      the dot sits on. But `windowStart` in the reactive chain uses
   *      `chromosomeRange.min` — the min across ALL selected genomes.
   *      If the earliest record on this chromosome came from a different
   *      genome, those two mins diverge and the landed window is shifted.
   *      The snap-to-nearest-hit step below corrects for that: after
   *      rebuilding `filteredWindows` against the cross-genome record
   *      set, we pick the filtered window closest to the estimate so
   *      the user always lands on a window that actually contains the
   *      searched contig.
   */
  async function navigateFromDot(chrId: number, estimatedWindow: number) {
    selectedChromosome = chrId;

    // Phase 1b: chromosome records come from the server-side cache.
    // If already cached, we can rebuild `filteredWindows` synchronously
    // for the target chromosome; otherwise we fetch (or let the main
    // reactive chain fetch it in the background) and fall back to the
    // estimated window as the target.
    //
    // In practice the overview panel only produces dots for chromosomes
    // that already have hits, so a cache entry usually exists by the
    // time the user clicks one. Cold clicks wait for the fetch.
    const key = cacheKey(selectedGenomes, chrId);
    let newChrRecords: WireAreaRecord[] = [];
    let newChrRange = { min: 0, max: 100000 };

    const cached = chromosomeRecordCache.get(key);
    if (cached) {
      newChrRecords = cached.records;
      newChrRange = getChromosomeRange(newChrRecords);
    } else if (sessionId && isQueryable) {
      try {
        const resp = await fetchChromosomeRecords(sessionId, {
          genomes: selectedGenomes,
          chr: chrId,
        });
        if (resp) {
          chromosomeRecordCache.set(key, resp);
          newChrRecords = resp.records;
          newChrRange = getChromosomeRange(newChrRecords);
        }
      } catch (err) {
        console.error('navigateFromDot fetch failed:', err);
      }
    }

    // Rebuild filteredWindows against the NEW chromosome if we're still
    // searching. Without this, nav buttons + the "N / M" indicator point
    // at the old chromosome's hit set.
    let targetWindow = estimatedWindow;
    if (isSearching && submittedSearchQuery) {
      const contigId = parseInt(submittedSearchQuery);
      if (!isNaN(contigId)) {
        filteredWindows = findWindowsWithContig(contigId, newChrRecords, newChrRange, windowSize);

        // Snap the landing index to the nearest actual hit window on this
        // chromosome. The dot's estimatedWindow is computed against the
        // single-genome rangeMin; the reactive chain uses a cross-genome
        // one. When those diverge (earliest record on this chromosome is
        // in a different genome than the row clicked), the estimate may
        // be one or two windows off. Snapping to the nearest real hit
        // window guarantees the user lands on a window that actually
        // contains the searched contig.
        if (filteredWindows.length > 0) {
          let nearest = filteredWindows[0];
          let bestDist = Math.abs(nearest - estimatedWindow);
          for (const w of filteredWindows) {
            const d = Math.abs(w - estimatedWindow);
            if (d < bestDist) {
              bestDist = d;
              nearest = w;
            }
          }
          targetWindow = nearest;
        }
      }
    }

    currentWindowIndex = targetWindow;

    areaAnalysisFilterState.update(state => ({
      ...state,
      selectedChromosome: chrId,
      currentWindowIndex: targetWindow,
    }));

    clearCaches();
  }

  /**
   * Window Sequence Comparison Panel state.
   *
   * This panel summarises, for the currently-visible window only:
   *   - contigs shared by ALL selected genomes
   *   - contigs unique to each genome
   * It's opt-in (expanded via toggle) because it recomputes on every
   * window navigation.
   */
  let comparisonPanelOpen = false;

  /**
   * Summary of which contigs are shared vs unique across the selected
   * genomes inside a single window. Powers the comparison panel UI.
   */
  interface SequenceComparison {
    /** Query contig IDs present in ALL selected genomes inside this window. */
    shared: number[];
    /** Per-genome breakdown of contigs that are NOT shared with all. */
    uniquePerGenome: { genomeIdx: number; genomeName: string; genomeColor: string; contigs: number[] }[];
    /** Total distinct contig IDs across all genomes in this window. */
    totalUnique: number;
  }

  /**
   * Build the comparison summary for the current window.
   *
   * Algorithm (all O(n) in records):
   *   1. For each selected genome, collect the Set of qry_contig_ids that
   *      have at least one record overlapping this window on this chromosome.
   *   2. "Shared" = intersection of those sets.
   *   3. "Unique per genome" = each set minus the shared set.
   *
   * If fewer than 2 genomes are selected the concept of "shared" is
   * meaningless, so we short-circuit to an empty result.
   */
  function buildWindowComparison(
    _records: WireAreaRecord[],
    _selectedGenomes: number[],
    _windowStart: number,
    _windowEnd: number,
    _files: FileData[],
  ): SequenceComparison {
    if (_selectedGenomes.length < 2) {
      return { shared: [], uniquePerGenome: [], totalUnique: 0 };
    }

    // genome index → Set of qry_contig_ids present in this window.
    const genomeContigs = new Map<number, Set<number>>();
    for (const gi of _selectedGenomes) {
      genomeContigs.set(gi, new Set());
    }

    // Phase 1b: `_records` is already filtered to the current
    // chromosome + selected genomes (it's `chromosomeRecords`). So we
    // only need to filter by window overlap here.
    for (const record of _records) {
      const gi = record.genome_index;
      if (!genomeContigs.has(gi)) continue;
      // Overlap test: any overlap with [_windowStart, _windowEnd] counts.
      if (record.ref_end_pos >= _windowStart && record.ref_start_pos <= _windowEnd) {
        genomeContigs.get(gi)!.add(record.qry_contig_id);
      }
    }

    // Shared = contigs present in ALL selected genome sets.
    const genomeSets = _selectedGenomes.map(gi => genomeContigs.get(gi)!);
    const allContigs = new Set<number>();
    for (const s of genomeSets) {
      for (const id of s) allContigs.add(id);
    }

    const shared: number[] = [];
    const sharedSet = new Set<number>();
    for (const id of allContigs) {
      if (genomeSets.every(s => s.has(id))) {
        shared.push(id);
        sharedSet.add(id);
      }
    }
    shared.sort((a, b) => a - b);

    // Unique = per-genome set MINUS the shared set, sorted for stable display.
    const uniquePerGenome = _selectedGenomes.map(gi => {
      const unique = Array.from(genomeContigs.get(gi)!)
        .filter(id => !sharedSet.has(id))
        .sort((a, b) => a - b);
      return {
        genomeIdx: gi,
        genomeName: _files[gi]?.name ?? `Genome ${gi}`,
        genomeColor: _files[gi]?.color ?? '#888',
        contigs: unique,
      };
    });

    return {
      shared,
      uniquePerGenome,
      totalUnique: allContigs.size,
    };
  }

  /**
   * Reactive wrapper: compute the comparison only when the panel is open.
   * Depends on windowStart / windowEnd defined further down; Svelte's
   * ordering rules handle the forward reference fine.
   */
  $: windowComparison = comparisonPanelOpen
    ? buildWindowComparison(chromosomeRecords, selectedGenomes, windowStart, windowEnd, files)
    : { shared: [], uniquePerGenome: [], totalUnique: 0 } as SequenceComparison;

  // ---------------------------------------------------------------------
  // CACHING LAYER 1 — Color cache.
  //
  // Colours are derived from contigId via the golden-ratio hue distribution
  // (hash ≈ contigId * 137.508 mod 360). Cheap individually, but called
  // many times per render; memoising avoids repeated float math.
  // ---------------------------------------------------------------------
  const colorCache = new Map<number, string>();
  
  /**
   * Generate a stable, visually-distinct HSL colour for a contigId.
   *
   * Multiplying by 137.508 (~golden angle × 2π / π) spreads successive IDs
   * around the hue wheel with minimal clustering. 70% sat / 60% lightness
   * keeps every colour readable on both light and dark backgrounds.
   */
  function generateContigColor(contigId: number): string {
    if (colorCache.has(contigId)) {
      return colorCache.get(contigId)!;
    }
    const hue = (contigId * 137.508) % 360;
    const color = `hsl(${hue}, 70%, 60%)`;
    colorCache.set(contigId, color);
    return color;
  }

  // ---------------------------------------------------------------------
  // CACHING LAYER 2 — Rendered contig bar cache.
  //
  // Each contig bar needs startX/endX/width/color/key computed from the
  // window bounds. Redoing that math on every render is wasteful when
  // filter state changes but the window bounds haven't. We cache the
  // whole computed-bar array and invalidate via (lastWindowStart,
  // lastWindowEnd, stackedContigs identity).
  // ---------------------------------------------------------------------
  interface CachedContigBar {
    record: any;
    startX: number;
    endX: number;
    width: number;
    color: string;
    key: string;
  }

  let cachedContigBars: CachedContigBar[][] = [];
  let lastWindowStart = -1;
  let lastWindowEnd = -1;
  let lastFilteredRecords: any[] = [];

  // ---------------------------------------------------------------------
  // CACHING LAYER 3 — Stacked-contig memoisation.
  //
  // stackContigs() is O(n × tracks) and called on every reactive tick.
  // We reuse the previous result when the (records reference, window
  // bounds) triple is unchanged — array identity (===) is the right test
  // here because filteredChromosomeRecords is only reconstructed when
  // its inputs change.
  // ---------------------------------------------------------------------
  let lastStackInputs: {
    records: any[];
    windowStart: number;
    windowEnd: number;
  } | null = null;
  let cachedStackedContigs: any[][] = [];

  /** Memoised wrapper around stackContigs(). */
  function getCachedStackedContigs(records: any[], windowStart: number, windowEnd: number): any[][] {
    if (
      lastStackInputs &&
      lastStackInputs.windowStart === windowStart &&
      lastStackInputs.windowEnd === windowEnd &&
      lastStackInputs.records === records  // reference equality, intentional
    ) {
      return cachedStackedContigs;
    }

    const stacked = stackContigs(records, windowStart, windowEnd);
    
    lastStackInputs = { records, windowStart, windowEnd };
    cachedStackedContigs = stacked;
    
    return stacked;
  }

  /**
   * Turn stacked contig tracks into cached bar geometry.
   *
   * Cache key: window bounds + the stackedContigs reference. If all three
   * match the previous call, return the existing array untouched — this
   * lets Svelte's `{#each}` block short-circuit on reference equality
   * and skip DOM work entirely.
   */
  function generateCachedContigBars(
    stackedContigs: any[][],
    windowStart: number,
    windowEnd: number,
    windowSize: number
  ): CachedContigBar[][] {
    if (
      lastWindowStart === windowStart &&
      lastWindowEnd === windowEnd &&
      stackedContigs === cachedStackedContigs
    ) {
      return cachedContigBars;
    }

    // Fresh computation: convert every record into a bar with pixel-space
    // coordinates. `posToX` handles clamping to [0..100].
    const newCache: CachedContigBar[][] = [];
    
    for (let trackIndex = 0; trackIndex < stackedContigs.length; trackIndex++) {
      const track = stackedContigs[trackIndex];
      const cachedTrack: CachedContigBar[] = [];
      
      for (let recordIndex = 0; recordIndex < track.length; recordIndex++) {
        const record = track[recordIndex];
        const startX = posToX(record.ref_start_pos, windowStart, windowSize);
        const endX = posToX(record.ref_end_pos, windowStart, windowSize);
        const width = endX - startX;
        const color = generateContigColor(record.qry_contig_id);
        // Composite key identifies this bar uniquely for {#each} keying —
        // file_index distinguishes the same contig on the same position
        // across genomes.
        const key = `${record.qry_contig_id}-${record.ref_start_pos}-${record.ref_end_pos}-${record.file_index}`;
        
        cachedTrack.push({ record, startX, endX, width, color, key });
      }
      
      newCache.push(cachedTrack);
    }

    lastWindowStart = windowStart;
    lastWindowEnd = windowEnd;
    lastFilteredRecords = stackedContigs.flat();
    cachedContigBars = newCache;

    return newCache;
  }

  /**
   * Invalidate every cache layer.
   *
   * Called whenever something upstream of the caches changes — different
   * chromosome, different genome selection, new search, etc. Cheaper to
   * blow the caches than to try to reason about which layer is still valid.
   */
  function clearCaches() {
    cachedContigBars = [];
    lastWindowStart = -1;
    lastWindowEnd = -1;
    lastFilteredRecords = [];
    lastStackInputs = null;
    cachedStackedContigs = [];
  }
  
  // ---------------------------------------------------------------------
  // Lazy-loading wiring (IntersectionObserver)
  // ---------------------------------------------------------------------
  let observer: IntersectionObserver;

  /**
   * Store-subscription handles; null until `initializeStoreSubscriptions`
   * wires them up (which only happens after the component becomes visible).
   * Kept in module scope so onDestroy can call them to unsubscribe.
   */
  let unsubscribeSearch: (() => void) | null = null;
  let unsubscribeFilter: (() => void) | null = null;

  /**
   * Wire up store subscriptions on first visibility.
   *
   * Two concerns are handled here:
   *
   *   1. searchStore — if the search text changed elsewhere (e.g. the
   *      user typed in DonutInfo's search which happens to use the same
   *      store key), we re-run `performSearch` to refresh our filtered
   *      windows. The `shouldReRunSearch` latch prevents double-running
   *      on the initial subscribe.
   *
   *   2. areaAnalysisFilterState — localStorage-persisted filter state.
   *      Pulls genome/chromosome/window/search into locals, then if a
   *      search is active, recomputes `filteredWindows` and nudges
   *      currentWindowIndex into the filtered set so we don't end up
   *      stranded on a window with no hits.
   *
   * The no-op guard (`if (unsubscribeSearch || unsubscribeFilter) return`)
   * makes this idempotent — subsequent calls are safe.
   */
  function initializeStoreSubscriptions() {
    if (unsubscribeSearch || unsubscribeFilter) return;

    unsubscribeSearch = searchStore.subscribe(state => {
      if (state.areaSearchQuery !== searchQuery) {
        searchQuery = state.areaSearchQuery;
        if (searchQuery.trim() && shouldReRunSearch) {
          performSearch(searchQuery.trim());
        }
      }
    });

    unsubscribeFilter = areaAnalysisFilterState.subscribe(state => {
      // selectedFiles in the store now holds GENOME indices (0, 1, 2);
      // the field name kept its old "files" label for back-compat with
      // serialized state in users' localStorage.
      selectedGenomes = state.selectedFiles;
      selectedChromosome = state.selectedChromosome;
      windowSize = state.windowSize;
      currentWindowIndex = state.currentWindowIndex;
      submittedSearchQuery = state.searchQuery || '';

      if (submittedSearchQuery.trim()) {
        isSearching = true;
        searchQuery = submittedSearchQuery;

        // Rebuild filteredWindows for the restored query.
        //
        // Phase 1b: records come from the chromosome-records cache. If
        // cached (main reactive chain already fetched for this key),
        // rebuild sync. Otherwise kick off an async rebuild; the
        // `chromosomeRecords` change fires its own reactives downstream.
        const contigId = parseInt(submittedSearchQuery.trim());
        if (!isNaN(contigId)) {
          (async () => {
            const key = cacheKey(selectedGenomes, selectedChromosome);
            let records: WireAreaRecord[] = [];
            const cached = chromosomeRecordCache.get(key);
            if (cached) {
              records = cached.records;
            } else if (sessionId && isQueryable) {
              try {
                const resp = await fetchChromosomeRecords(sessionId, {
                  genomes: selectedGenomes,
                  chr: selectedChromosome,
                });
                if (resp) {
                  chromosomeRecordCache.set(key, resp);
                  records = resp.records;
                }
              } catch (err) {
                console.error('Restored-search records fetch failed:', err);
              }
            }
            const chromosomeRange = getChromosomeRange(records);
            filteredWindows = findWindowsWithContig(contigId, records, chromosomeRange, windowSize);

            if (filteredWindows.length > 0) {
              if (!filteredWindows.includes(currentWindowIndex)) {
                // Update locally immediately so downstream reactives see the
                // correct window in the same tick (avoids a one-frame flash
                // of "no occurrences in this window").
                currentWindowIndex = filteredWindows[0];
                areaAnalysisFilterState.update(s => ({
                  ...s,
                  currentWindowIndex: currentWindowIndex
                }));
              }
            } else {
              // No hits at all — snap to window 0 so the user sees something
              // reasonable when they clear the search.
              if (currentWindowIndex !== 0) {
                currentWindowIndex = 0;
                areaAnalysisFilterState.update(s => ({
                  ...s,
                  currentWindowIndex: 0
                }));
              }
            }
          })();
        }
      } else {
        isSearching = false;
        filteredWindows = [];
      }
    });
  }

  /**
   * Mount handler: set up IntersectionObserver so heavy work defers until
   * the component actually scrolls into view.
   *
   * rootMargin: '50px' — start waking up slightly before the component
   * crosses the viewport edge, so the user sees content immediately on
   * scroll rather than a frame of empty space.
   *
   * threshold: 0.1 — only 10% of the component needs to be visible to
   * trigger. Lower than the default (0) guards against spurious fires
   * from layout shifts.
   */
  onMount(() => {
    observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting && !isInitialized) {
            isVisible = true;
            isInitialized = true;
            initializeStoreSubscriptions();
            shouldReRunSearch = true;
            
            // One-shot subscription to re-run the persisted search. We
            // unsubscribe immediately after the first callback so the
            // normal filter-state subscription (inside
            // initializeStoreSubscriptions) takes over from here.
            const unsubscribeInitial = areaAnalysisFilterState.subscribe((state) => {
              if (state.searchQuery && state.searchQuery.trim()) {
                searchQuery = state.searchQuery;
                performSearch(state.searchQuery.trim());
              }
              unsubscribeInitial();
            });
          }
        });
      },
      { root: null, rootMargin: '50px', threshold: 0.1 }
    );

    if (containerElement) {
      observer.observe(containerElement);
    }
  });

  /**
   * Run a contig search.
   *
   * Only numeric queries make sense (contig IDs are integers); a
   * non-numeric query is silently ignored — the UI doesn't error,
   * the user just sees no filter applied.
   *
   * Window-index update is done LOCALLY first, before the store push,
   * so the reactive chain (windowStart → stackedContigs →
   * renderedContigBars) picks up the new window inside the same tick.
   * Without that you'd see a flash of "no occurrences" before the store
   * round-trip completes.
   */
  function performSearch(query: string) {
    if (!query.trim()) {
      resetSearch();
      return;
    }

    const contigId = parseInt(query.trim());
    if (!isNaN(contigId)) {
      submittedSearchQuery = query.trim();
      isSearching = true;

      // Phase 1b: use the already-fetched `chromosomeRecords` rather
      // than recomputing from `matches` (which is empty). If the user
      // hasn't switched chromosome since the last fetch, this array
      // is current. If they just switched, the main reactive chain has
      // already kicked off a fetch and filteredWindows will be rebuilt
      // when the restored-search subscribe path runs (with cache-hit
      // this happens on the same tick).
      const range = getChromosomeRange(chromosomeRecords);
      filteredWindows = findWindowsWithContig(contigId, chromosomeRecords, range, windowSize);

      // Pick the target window: if the current one is already in the hit
      // set, stay put; otherwise jump to the first hit. Everything falls
      // back to window 0 when there are no hits.
      const targetWindowIndex = filteredWindows.length > 0
        ? (filteredWindows.includes(currentWindowIndex) ? currentWindowIndex : filteredWindows[0])
        : 0;
      currentWindowIndex = targetWindowIndex;

      areaAnalysisFilterState.update(state => ({
        ...state,
        searchQuery: submittedSearchQuery,
        currentWindowIndex: targetWindowIndex
      }));

      searchStore.update(state => ({ ...state, areaSearchQuery: query }));
      clearCaches();
    }
  }

  /** Submit handler for the search input (Enter key or button press). */
  function handleSearch() {
    performSearch(searchQuery.trim());
  }

  /**
   * Filter records by selected GENOMES (not flat file indices).
   * Translates record.file_index → genome index via fileToGenome before filtering.
   */
  // Phase 1b: `getRecordsForChromosome` removed. Its function (walk all
  // matches, filter by (genomes, chromosome), dedup) is now performed
  // server-side by `/api/session/:id/chromosome-records`. Records arrive
  // already filtered and deduped.

  /**
   * Find the min/max reference positions across a record set.
   *
   * Used to compute `chromosomeRange` — the span that windows pagination
   * operates over. Falls back to [0..100000] for empty sets so the UI
   * still has something to render.
   */
  function getChromosomeRange(records: any[]) {
    if (records.length === 0) return { min: 0, max: 100000 };
    const min = Math.min(...records.map(r => r.ref_start_pos));
    const max = Math.max(...records.map(r => r.ref_end_pos));
    return { min: Math.floor(min), max: Math.ceil(max) };
  }

  /**
   * Stack overlapping contig alignments into parallel tracks.
   *
   * Classic interval-scheduling greedy algorithm:
   *   1. Filter to records overlapping the current window.
   *   2. Sort by start position.
   *   3. For each record, try to drop it into an existing track where it
   *      doesn't overlap anything. If none fits, start a new track.
   *
   * The result is the minimum number of tracks needed to display every
   * alignment without overlap — what the user sees as horizontal rows.
   *
   * Complexity: O(N × T) where N = records and T = resulting tracks.
   * In practice T is small (dozens at most) so this is fine; the
   * caching layer above keeps us from re-running it unnecessarily.
   */
  function stackContigs(records: any[], windowStart: number, windowEnd: number) {
    // (1) filter — the caller may pass the full chromosome; we only want
    // alignments that touch the current window.
    const visibleRecords = records.filter(r => 
      r.ref_end_pos >= windowStart && r.ref_start_pos <= windowEnd
    );

    // (2) sort ascending by start. Greedy track-assignment needs this
    // order to behave correctly.
    visibleRecords.sort((a, b) => a.ref_start_pos - b.ref_start_pos);

    // (3) greedy placement.
    const stacked: any[][] = [];
    for (const record of visibleRecords) {
      let placed = false;
      for (let trackIndex = 0; trackIndex < stacked.length; trackIndex++) {
        const track = stacked[trackIndex];
        
        // Half-open overlap test: A and B overlap iff A.start < B.end && A.end > B.start.
        let hasOverlap = false;
        for (const existingRecord of track) {
          if (record.ref_start_pos < existingRecord.ref_end_pos && 
              record.ref_end_pos > existingRecord.ref_start_pos) {
            hasOverlap = true;
            break;
          }
        }
        
        if (!hasOverlap) {
          track.push(record);
          placed = true;
          break;
        }
      }
      
      // No existing track fits — open a new one.
      if (!placed) {
        stacked.push([record]);
      }
    }

    return stacked;
  }

  /**
   * Convert a genomic position to a percentage within the current window.
   *
   * Returns 0..100, clamped. Used directly as a CSS `left`/`width`
   * percentage on the bars.
   */
  function posToX(pos: number, windowStart: number, windowSize: number): number {
    const relativePos = pos - windowStart;
    const percentage = (relativePos / windowSize) * 100;
    return Math.max(0, Math.min(100, percentage));
  }

  /**
   * Clamp currentWindowIndex into the valid range for a hypothetical
   * future genome selection, WITHOUT forcing a reset to 0.
   *
   * Used before toggling genomes so the user stays on their current window
   * unless that window index no longer exists (because the new selection
   * has fewer windows). Keeping position is a better UX than "every click
   * resets your scroll".
   *
   * Phase 1b: uses `chromosomeInfo` rather than a record scan to know the
   * chromosome's length. Chromosomes have a fixed reference length (it's
   * a property of the reference, not the selection), so any of the new
   * genomes' chromosomeInfo entries works.
   */
  function clampWindowIndex(newGenomes: number[]): number {
    if (newGenomes.length === 0) return 0;
    // Find the chromosome's length from any of the selected genomes'
    // chromosomeInfo. In practice all entries agree (same reference),
    // but we defensively take the max just in case.
    let refLen = windowSize;
    for (const gi of newGenomes) {
      const chrs = chromosomeInfo[gi] ?? [];
      for (const c of chrs) {
        if (c.ref_contig_id === selectedChromosome && c.ref_len > refLen) {
          refLen = c.ref_len;
        }
      }
    }
    const newTotalWindows = Math.ceil(refLen / windowSize);
    return Math.min(currentWindowIndex, Math.max(0, newTotalWindows - 1));
  }

  /**
   * Toggle one genome in the selection.
   *
   * Adds or removes the genome index, preserves the user's window position
   * if possible (clampWindowIndex), then pushes the change to the store
   * and clears caches so the next render is fresh.
   */
  function toggleGenomeSelection(genomeIndex: number) {
    let newSelectedGenomes: number[];
    
    if (selectedGenomes.includes(genomeIndex)) {
      newSelectedGenomes = selectedGenomes.filter(i => i !== genomeIndex);
    } else {
      // Sorted for stable display order (checkbox rows vs data rows).
      newSelectedGenomes = [...selectedGenomes, genomeIndex].sort((a, b) => a - b);
    }
    
    const clampedIndex = clampWindowIndex(newSelectedGenomes);
    selectedGenomes = newSelectedGenomes;
    currentWindowIndex = clampedIndex;
    
    areaAnalysisFilterState.update(state => ({ 
      ...state, 
      selectedFiles: newSelectedGenomes,
      currentWindowIndex: clampedIndex
    }));
    
    clearCaches();
    // Reset the search because the hit set depends on selected genomes.
    // Pass `clampedIndex` so we stay on the same window rather than
    // jumping to 0.
    resetSearch(clampedIndex);
  }

  /** "Select All" button: include every genome that was uploaded. */
  function selectAllGenomes() {
    const all = files.map((_, idx) => idx);
    const clampedIndex = clampWindowIndex(all);
    selectedGenomes = all;
    currentWindowIndex = clampedIndex;
    
    areaAnalysisFilterState.update(state => ({ 
      ...state, 
      selectedFiles: all,
      currentWindowIndex: clampedIndex
    }));
    
    clearCaches();
    resetSearch(clampedIndex);
  }

  /** "Clear All" button: no genomes selected (will show empty viewport). */
  function clearGenomeSelection() {
    selectedGenomes = [];
    currentWindowIndex = 0;
    
    areaAnalysisFilterState.update(state => ({ 
      ...state, 
      selectedFiles: [],
      currentWindowIndex: 0
    }));
    
    clearCaches();
    resetSearch();
  }

  /**
   * Blow away the current search and (optionally) preserve window position.
   *
   * `preservedWindowIndex` defaults to 0 — i.e. most callers want a full
   * reset. Genome-selection callers pass in a clamped index so the user's
   * scroll position survives the genome toggle.
   */
  function resetSearch(preservedWindowIndex: number = 0) {
    searchQuery = '';
    submittedSearchQuery = '';
    isSearching = false;
    filteredWindows = [];
    currentWindowIndex = preservedWindowIndex;
    
    searchStore.update(state => ({ ...state, areaSearchQuery: '' }));
    areaAnalysisFilterState.update(state => ({ 
      ...state, 
      searchQuery: '',
      currentWindowIndex: preservedWindowIndex
    }));
    
    clearCaches();
  }

  /**
   * Compute which windows contain at least one hit for the given contigId.
   *
   * A record that spans multiple windows contributes to all of them — we
   * take the floor of both start and end positions in window units and
   * add every index in between to the Set.
   */
  function findWindowsWithContig(contigId: number, records: any[], chromosomeRange: any, windowSize: number): number[] {
    const contigRecords = records.filter(record => record.qry_contig_id === contigId);
    if (contigRecords.length === 0) return [];

    const windowsWithContig = new Set<number>();
    
    for (const record of contigRecords) {
      const startWindow = Math.floor((record.ref_start_pos - chromosomeRange.min) / windowSize);
      const endWindow = Math.floor((record.ref_end_pos - chromosomeRange.min) / windowSize);
      
      // Include every window in the span (inclusive).
      for (let windowIndex = startWindow; windowIndex <= endWindow; windowIndex++) {
        if (windowIndex >= 0) {
          windowsWithContig.add(windowIndex);
        }
      }
    }
    
    return Array.from(windowsWithContig).sort((a, b) => a - b);
  }

  /** Enter = submit, Escape = clear. Standard search-box keybindings. */
  function handleSearchKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      handleSearch();
    } else if (e.key === 'Escape') {
      resetSearch();
    }
  }

  /** UI "×" button inside the search field. */
  function clearSearch() {
    resetSearch();
  }

  /**
   * Next / previous window navigation.
   *
   * When NOT searching, we just +/- 1 on the raw index. When searching,
   * we walk through `filteredWindows` (the subset of windows containing
   * hits for the search contig) so the user can hop between occurrences.
   */
  function goToNextWindow() {
    if (isSearching && filteredWindows.length > 0) {
      const currentIndexInFiltered = filteredWindows.indexOf(currentWindowIndex);
      if (currentIndexInFiltered < filteredWindows.length - 1) {
        currentWindowIndex = filteredWindows[currentIndexInFiltered + 1];
        areaAnalysisFilterState.update(state => ({ ...state, currentWindowIndex: currentWindowIndex }));
      }
    } else {
      currentWindowIndex++;
      areaAnalysisFilterState.update(state => ({ ...state, currentWindowIndex: currentWindowIndex }));
    }
  }

  /** Mirror of goToNextWindow in the other direction. */
  function goToPrevWindow() {
    if (isSearching && filteredWindows.length > 0) {
      const currentIndexInFiltered = filteredWindows.indexOf(currentWindowIndex);
      if (currentIndexInFiltered > 0) {
        currentWindowIndex = filteredWindows[currentIndexInFiltered - 1];
        areaAnalysisFilterState.update(state => ({ ...state, currentWindowIndex: currentWindowIndex }));
      }
    } else {
      currentWindowIndex--;
      areaAnalysisFilterState.update(state => ({ ...state, currentWindowIndex: currentWindowIndex }));
    }
  }

  // ---------------------------------------------------------------------
  // Reactive chain that produces everything the template renders.
  //
  // All gated on `isVisible` — the IntersectionObserver flips that to
  // true once the component scrolls into view. Before then every reactive
  // evaluates to an empty array, so the page pays nothing for this tab
  // if the user doesn't open it.
  //
  // The chain flows:
  //   chromosomeRecords → filteredChromosomeRecords → stackedContigs
  //   → renderedContigBars (the bars we actually draw)
  // ---------------------------------------------------------------------

  // Note: `chromosomeRecords` is no longer a reactive `$:` derivation.
  // It's populated by `reloadChromosomeRecords()` above (async fetch
  // with cache). The template still reads it the same way, so downstream
  // reactives (`filteredChromosomeRecords`, `chromosomeRange`, etc.)
  // still recompute when the value changes.

  /** Same list, further filtered by the search contig if active. */
  $: filteredChromosomeRecords = isSearching
    ? chromosomeRecords.filter(record => {
        const contigId = parseInt(submittedSearchQuery);
        if (isNaN(contigId)) return false;
        return record.qry_contig_id === contigId;
      })
    : chromosomeRecords;
  /** Min/max bp positions across this chromosome's records. */
  $: chromosomeRange = getChromosomeRange(chromosomeRecords);
  /**
   * Chromosome length in bp. Previously taken from the first record's
   * `ref_len`; now comes from the server response directly
   * (`chromosomeRefLenFetched`) so it's correct even for empty result
   * sets. Falls back to `windowSize` to avoid divide-by-zero downstream.
   */
  $: chromosomeRefLen = chromosomeRefLenFetched > 0
    ? chromosomeRefLenFetched
    : (chromosomeRecords.length > 0 ? chromosomeRecords[0].ref_len : windowSize);
  
  /** Total number of windows that tile the chromosome. */
  $: totalWindows = Math.ceil(chromosomeRefLen / windowSize);
  /** Total windows shown to the user — filtered subset when searching. */
  $: effectiveTotalWindows = isSearching ? filteredWindows.length : totalWindows;
  /**
   * 1-based "page number" for display. Either the 1-based index within
   * filteredWindows when searching, or the raw 1-based window index.
   */
  $: effectiveCurrentWindowIndex = isSearching ? 
    (filteredWindows.indexOf(currentWindowIndex) + 1 || 1) : 
    (currentWindowIndex + 1);
  
  /** bp bounds of the current window. Clamped to chromosome length. */
  $: windowStart = chromosomeRange.min + (currentWindowIndex * windowSize);
  $: windowEnd = Math.min(windowStart + windowSize, chromosomeRefLen);
  
  /** Memoised stacked tracks for this window. */
  $: stackedContigs = isVisible ? getCachedStackedContigs(filteredChromosomeRecords, windowStart, windowEnd) : [];
  /** Memoised bar geometry — what the template iterates over. */
  $: renderedContigBars = isVisible ? generateCachedContigBars(stackedContigs, windowStart, windowEnd, windowSize) : [];
  /** Sorted unique contig IDs — drives the legend list. */
  $: uniqueContigs = Array.from(new Set(filteredChromosomeRecords.map(r => r.qry_contig_id))).sort((a, b) => a - b);
  
  /** Prev/next button enable states, accounting for search mode. */
  $: canGoPrev = isSearching ? 
    filteredWindows.indexOf(currentWindowIndex) > 0 : 
    currentWindowIndex > 0;
  $: canGoNext = isSearching ? 
    filteredWindows.indexOf(currentWindowIndex) < filteredWindows.length - 1 : 
    currentWindowIndex < totalWindows - 1;

  /** Chromosome dropdown options — 1..24 (22 autosomes + X=23 + Y=24). */
  const chromosomes = Array.from({ length: 24 }, (_, i) => i + 1);

  /**
   * Chromosome dropdown change handler.
   *
   * Switching chromosome fundamentally changes the data being shown, so:
   *   - jump to window 0 (the user's old window index has no meaning in
   *     the new chromosome),
   *   - clear caches,
   *   - if there's an active search, re-run it against the new chromosome.
   */
  function handleChromosomeChange() {
    currentWindowIndex = 0;
    
    areaAnalysisFilterState.update(state => ({ 
      ...state, 
      selectedChromosome: selectedChromosome,
      currentWindowIndex: 0
    }));
    
    clearCaches();
    
    if (isSearching && submittedSearchQuery) {
      performSearch(submittedSearchQuery);
    }
  }

  /**
   * Click-to-edit state for the "page N of M" indicator.
   * When editing, the static label is replaced by a numeric input.
   */
  let editingWindowPage = false;
  let windowPageInput = '';

  /** Activate the input. */
  function startEditingWindowPage() {
    editingWindowPage = true;
    windowPageInput = effectiveCurrentWindowIndex.toString();
  }

  /**
   * Commit a manual window jump. Clamps to valid range; in search mode
   * the number the user typed is interpreted as "Nth filtered window",
   * not the raw chromosome window index.
   */
  function submitWindowPageJump() {
    const pageNum = parseInt(windowPageInput);
    if (!isNaN(pageNum)) {
      if (isSearching && filteredWindows.length > 0) {
        const newFilteredIndex = Math.max(0, Math.min(pageNum - 1, filteredWindows.length - 1));
        currentWindowIndex = filteredWindows[newFilteredIndex];
      } else {
        const newIndex = Math.max(0, Math.min(pageNum - 1, totalWindows - 1));
        currentWindowIndex = newIndex;
      }
      areaAnalysisFilterState.update(state => ({ ...state, currentWindowIndex: currentWindowIndex }));
    }
    editingWindowPage = false;
  }

  /** Enter submits, Escape cancels. */
  function handleWindowPageKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      submitWindowPageJump();
    } else if (e.key === 'Escape') {
      editingWindowPage = false;
    }
  }

  /**
   * Flat lookup map from contig bar key → record. Enables event delegation:
   * we attach ONE mousemove handler on the viewport container instead of
   * one per bar, and use `closest('[data-contig-key]')` to identify the
   * hovered bar. That drops from O(N) listeners to O(1) — critical when
   * N can be thousands of bars.
   */
  let contigKeyMap = new Map<string, any>();
  $: {
    contigKeyMap = new Map();
    for (const track of renderedContigBars) {
      for (const bar of track) {
        contigKeyMap.set(bar.key, bar.record);
      }
    }
  }

  /**
   * Delegated mousemove handler: walk up the DOM from event.target to
   * find the nearest element carrying `data-contig-key`, then look up
   * the corresponding record. Reference-compares to avoid triggering
   * reactive updates when the hovered record hasn't actually changed.
   */
  function handleContigMouseMove(e: MouseEvent) {
    const target = (e.target as HTMLElement).closest('[data-contig-key]') as HTMLElement | null;
    if (!target) {
      if (hoveredContig !== null) hoveredContig = null;
      return;
    }
    const key = target.dataset.contigKey!;
    const record = contigKeyMap.get(key) ?? null;
    if (hoveredContig !== record) hoveredContig = record;
  }

  /** Mouse-leave on the viewport: clear any lingering hover state. */
  function handleContigMouseLeave() {
    hoveredContig = null;
  }

  /**
   * Component teardown: release every resource we might have claimed.
   *
   * If the user never scrolled the tab into view, most of these are no-ops
   * (the subscriptions/observer were never set up). The guards make
   * running this cheap in that case.
   */
  onDestroy(() => {
    if (unsubscribeSearch) unsubscribeSearch();
    if (unsubscribeFilter) unsubscribeFilter();
    if (observer) observer.disconnect();
    clearCaches();
    colorCache.clear();
  });
</script>

<div class="analysis-container" bind:this={containerElement}>
  {#if !isVisible}
    <div class="lazy-placeholder">
      <div class="lazy-spinner"></div>
      <p>Loading Area Analysis...</p>
    </div>
  {:else}
    <!-- Genome selection controls -->
    <div class="controls">
      <div class="control-group full-width">
        <label for="genome-selection">Select Genomes:</label>
        <div class="file-selection" id="genome-selection">
          {#each files as genome, idx}
            <label class="file-checkbox">
              <input 
                type="checkbox" 
                checked={selectedGenomes.includes(idx)}
                on:change={() => toggleGenomeSelection(idx)}
              />
              <span class="file-checkbox-label">
                <span class="file-color-indicator" style="background: {genome.color}"></span>
                {genome.name}
              </span>
            </label>
          {/each}
        </div>
        <div class="file-selection-actions">
          <button class="action-btn" on:click={selectAllGenomes}>Select All</button>
          <button class="action-btn" on:click={clearGenomeSelection}>Clear All</button>
          <span class="selected-count">{selectedGenomes.length} of {files.length} selected</span>
        </div>
      </div>

      <div class="control-group">
        <label for="chromosome-select">Select Chromosome:</label>
        <select id="chromosome-select" bind:value={selectedChromosome} on:change={handleChromosomeChange}>
          {#each chromosomes as chr}
            <option value={chr}>Chromosome {chr}</option>
          {/each}
        </select>
      </div>
    </div>

    {#if uniqueContigs.length > 0}
      <div class="legend">
        <div class="legend-header">
          <h3>
            {#if isSearching}
              Showing Contig {submittedSearchQuery} ({filteredWindows.length} windows)
            {:else}
              Query Contigs ({uniqueContigs.length})
            {/if}
          </h3>
          <div class="search-bar">
            <input
              type="text"
              placeholder="Search contig ID and press Enter..."
              bind:value={searchQuery}
              on:keydown={handleSearchKeydown}
              class="search-input"
            />
          </div>
        </div>
        {#if !isSearching}
          <div class="legend-items">
            {#each uniqueContigs as contigId}
              <div class="legend-item">
                <div class="legend-color" style="background: {generateContigColor(contigId)}"></div>
                <span>QryContig {contigId}</span>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {:else if selectedGenomes.length > 0}
      <div class="legend">
        <div class="legend-header">
          <h3>No Contigs Found</h3>
          <div class="search-bar">
            <input
              type="text"
              placeholder="Search contig ID and press Enter..."
              bind:value={searchQuery}
              on:keydown={handleSearchKeydown}
              class="search-input"
            />
          </div>
        </div>
      </div>
    {/if}

    <div class="overview-panel" class:open={overviewPanelOpen}>
      <button
        class="overview-toggle"
        on:click={() => overviewPanelOpen = !overviewPanelOpen}
        aria-expanded={overviewPanelOpen}
      >
        <svg
          class="toggle-chevron"
          class:rotated={overviewPanelOpen}
          width="16" height="16" viewBox="0 0 16 16" fill="none"
        >
          <path d="M6 4l4 4-4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        <span class="overview-toggle-title">Chromosome Overview</span>
        {#if isSearching && submittedSearchQuery}
          <span class="overview-search-badge">Contig {submittedSearchQuery}</span>
        {/if}
      </button>

      {#if overviewPanelOpen}
        <div class="overview-body">
          {#if overviewData.length === 0}
            <p class="overview-empty">No chromosome data available.</p>
          {:else}
            {#each overviewData as genome, gi}
              <div class="overview-genome">
                <div class="overview-genome-header">
                  <span class="overview-genome-dot" style="background: {genome.genomeColor}"></span>
                  <span class="overview-genome-name">{genome.genomeName}</span>
                </div>
                <div class="overview-lines">
                  {#each genome.lines as line}
                    <div class="overview-chr-row">
                      <span class="overview-chr-label">Chr {line.chrId}</span>
                      <div class="overview-chr-track">
                        <div class="overview-line-bg"></div>
                        {#each line.markers as markerBp, mi}
                          {@const pct = line.refLen > 0 ? (markerBp / line.refLen) * 100 : 0}
                          <div
                            class="overview-marker"
                            class:overview-marker-end={mi === 0 || mi === line.markers.length - 1}
                            style="left: {pct}%"
                            title="{markerBp.toLocaleString()} bp"
                          >
                            <span class="overview-marker-label" class:overview-marker-label-end={mi === 0 || mi === line.markers.length - 1}>
                              {#if markerBp >= 1e6}
                                {(markerBp / 1e6).toFixed(0)}M
                              {:else if markerBp >= 1e3}
                                {(markerBp / 1e3).toFixed(0)}k
                              {:else}
                                {markerBp}
                              {/if}
                            </span>
                          </div>
                        {/each}
                        {#each line.dots as dot}
                          {@const winBpStart = dot.estimatedWindow * windowSize}
                          {@const winBpEnd = Math.min(line.refLen, winBpStart + windowSize)}
                          {@const bpLabel = `${(winBpStart / 1e3).toFixed(0)}–${(winBpEnd / 1e3).toFixed(0)} kb`}
                          <button
                            class="overview-dot"
                            style="left: {dot.xFraction * 100}%"
                            title="Window {dot.estimatedWindow + 1} ({bpLabel})"
                            on:click={() => navigateFromDot(line.chrId, dot.estimatedWindow)}
                          >
                            <!--
                              Hover tooltip. Shows the window number and
                              its bp range in kB for a quick positional
                              estimate before clicking.
                            -->
                            <span class="overview-dot-tooltip">
                              Window {dot.estimatedWindow + 1}<br/>
                              {bpLabel}
                            </span>
                          </button>
                        {/each}
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {/each}
          {/if}
        </div>
      {/if}
    </div>

    <div class="comparison-panel" class:open={comparisonPanelOpen}>
      <button
        class="overview-toggle"
        on:click={() => comparisonPanelOpen = !comparisonPanelOpen}
        aria-expanded={comparisonPanelOpen}
      >
        <svg
          class="toggle-chevron"
          class:rotated={comparisonPanelOpen}
          width="16" height="16" viewBox="0 0 16 16" fill="none"
        >
          <path d="M6 4l4 4-4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        <span class="overview-toggle-title">Window Sequence Comparison</span>
        {#if windowComparison.totalUnique > 0}
          <span class="comparison-count-badge">
            {windowComparison.shared.length} shared · {windowComparison.totalUnique} total
          </span>
        {/if}
      </button>

      {#if comparisonPanelOpen}
        <div class="comparison-body">
          {#if selectedGenomes.length < 2}
            <p class="overview-empty">Select at least 2 genomes to compare sequences.</p>
          {:else if windowComparison.totalUnique === 0}
            <p class="overview-empty">No sequences found in this window.</p>
          {:else}
            <div class="comparison-section">
              <div class="comparison-section-header">
                <span class="comparison-section-title">Shared across all genomes</span>
                <span class="comparison-section-count">{windowComparison.shared.length}</span>
              </div>
              {#if windowComparison.shared.length > 0}
                <div class="legend-items">
                  {#each windowComparison.shared as contigId}
                    <div class="legend-item">
                      <div class="legend-color" style="background: {generateContigColor(contigId)}"></div>
                      <span>QryContig {contigId}</span>
                    </div>
                  {/each}
                </div>
              {:else}
                <p class="comparison-none">No shared sequences in this window.</p>
              {/if}
            </div>

            {#each windowComparison.uniquePerGenome as genomeGroup}
              <div class="comparison-section">
                <div class="comparison-section-header">
                  <span class="comparison-genome-dot" style="background: {genomeGroup.genomeColor}"></span>
                  <span class="comparison-section-title">Only in {genomeGroup.genomeName}</span>
                  <span class="comparison-section-count">{genomeGroup.contigs.length}</span>
                </div>
                {#if genomeGroup.contigs.length > 0}
                  <div class="legend-items">
                    {#each genomeGroup.contigs as contigId}
                      <div class="legend-item">
                        <div class="legend-color" style="background: {generateContigColor(contigId)}"></div>
                        <span>QryContig {contigId}</span>
                      </div>
                    {/each}
                  </div>
                {:else}
                  <p class="comparison-none">No unique sequences.</p>
                {/if}
              </div>
            {/each}
          {/if}
        </div>
      {/if}
    </div>

    <div class="window-info">
      <div class="window-position">
        <strong>Window:</strong> {windowStart.toLocaleString()} - {windowEnd.toLocaleString()} bp
        {#if isSearching}
          <span class="search-indicator">(Searching: Contig {submittedSearchQuery})</span>
        {/if}
        {#if editingWindowPage}
          <input
            type="text"
            class="window-page-input"
            bind:value={windowPageInput}
            on:keydown={handleWindowPageKeydown}
            on:blur={submitWindowPageJump}
            on:focus
          />
        {:else}
          <span 
            class="window-count" 
            on:dblclick={startEditingWindowPage}
            role="button"
            tabindex="0"
            on:keydown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                startEditingWindowPage();
              }
            }}
          >
            ({effectiveCurrentWindowIndex} / {effectiveTotalWindows})
            {#if isSearching && filteredWindows.length > 0}
              <span class="filtered-pages">(Filtered)</span>
            {/if}
          </span>
        {/if}
      </div>
      <div class="window-navigation">
        <button on:click={goToPrevWindow} disabled={!canGoPrev}>
          ← Previous
        </button>
        <button on:click={goToNextWindow} disabled={!canGoNext}>
          Next →
        </button>
      </div>
    </div>

    <div class="browser">
      {#if selectedGenomes.length === 0}
        <div class="empty-state">
          No genomes selected. Please select one or more genomes to view mappings.
        </div>
      {:else if chromosomeRecords.length === 0}
        <div class="empty-state">
          No mappings found for this chromosome in selected genomes.
        </div>
      {:else if renderedContigBars.length === 0}
        <div class="empty-state">
          {#if isSearching}
            No occurrences of Contig {submittedSearchQuery} in this window
          {:else}
            No mappings in this window. Use navigation buttons to explore other regions.
          {/if}
        </div>
      {:else}
         <div
          class="browser-inner"
          on:mousemove={handleContigMouseMove}
          on:mouseleave={handleContigMouseLeave}
          role="presentation"
        >
        <div class="position-markers">
          {#each [0, 0.25, 0.5, 0.75, 1] as fraction}
            {@const pos = windowStart + (windowSize * fraction)}
            {#if pos <= windowEnd}
              <div class="marker" style="left: {fraction * 100}%">
                <div class="marker-tick"></div>
                <div class="marker-label">{Math.round(pos).toLocaleString()}</div>
              </div>
            {/if}
          {/each}
        </div>

        <!-- Scrollable seq area w/ CACHED BARS -->
        <!-- Event delegation moved to browser-inner so sticky position-markers
             doesn't create a gap where mousemove events are lost -->
        <div
          class="contigs-viewport"
          role="presentation"
        >
          <div class="contigs-container">
            {#each renderedContigBars as track, trackIndex (trackIndex)}
              <div class="contig-track">
                {#each track as cachedBar (cachedBar.key)}
                  <div
                    class="contig"
                    class:hovered={hoveredContig === cachedBar.record}
                    style="left: {cachedBar.startX}%; width: {cachedBar.width}%; background: {cachedBar.color}"
                    data-contig-key={cachedBar.key}
                    role="button"
                    tabindex="0"
                  ></div>
                {/each}
              </div>
            {/each}
          </div>
        </div>
        </div>

        <!-- Hover tooltip w/detailed seq info -->
        {#if hoveredContig}
          {@const genomeIdx = fileToGenome[hoveredContig.file_index] ?? 0}
          <div class="tooltip">
            <div class="tooltip-header">
              Query Contig {hoveredContig.qry_contig_id}
            </div>
            <div class="tooltip-body">
              <div class="tooltip-file">
                <span class="file-badge" style="background: {files[genomeIdx]?.color}20; color: {files[genomeIdx]?.color}; border-color: {files[genomeIdx]?.color}">
                  {files[genomeIdx]?.name}
                </span>
              </div>
              <div class="tooltip-content">
                <div><strong>Ref Position:</strong> {hoveredContig.ref_start_pos.toLocaleString()} - {hoveredContig.ref_end_pos.toLocaleString()} bp</div>
                <div><strong>Query Position:</strong> {hoveredContig.qry_start_pos.toLocaleString()} - {hoveredContig.qry_end_pos.toLocaleString()} bp</div>
                <div><strong>Orientation:</strong> {hoveredContig.orientation}</div>
                <div><strong>Confidence:</strong> {hoveredContig.confidence.toFixed(2)}</div>
                <div><strong>Ref Length:</strong> {(hoveredContig.ref_end_pos - hoveredContig.ref_start_pos).toLocaleString()} bp</div>
                <div><strong>Query Length:</strong> {(hoveredContig.qry_end_pos - hoveredContig.qry_start_pos).toLocaleString()} bp</div>
              </div>
            </div>
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .analysis-container {
    padding: 2rem;
    max-width: 1400px;
    margin: 0 auto;
    min-height: 400px;
  }

  .lazy-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1rem;
    padding: 4rem;
    background: var(--bg-secondary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
    min-height: 400px;
  }

  .lazy-spinner {
    width: 40px;
    height: 40px;
    border: 4px solid var(--border-color);
    border-top-color: var(--accent-primary);
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .lazy-placeholder p {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .controls {
    display: grid;
    grid-template-columns: 1fr;
    gap: 1rem;
    margin-bottom: 2rem;
    padding: 1.5rem;
    background: var(--bg-secondary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
  }
  .control-group { display: flex; flex-direction: column; gap: 0.5rem; }
  .control-group.full-width { grid-column: 1 / -1;
  }
  .control-group label {
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--text-primary);
  }
  .control-group select {
    padding: 0.5rem;
    border-radius: 0.375rem;
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 0.875rem;
  }

  .file-selection {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin-top: 0.5rem;
  }

  .file-checkbox {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    cursor: pointer;
    padding: 0.5rem 0.75rem;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 0.375rem;
  }
  .file-checkbox input[type="checkbox"] { cursor: pointer; }

  .file-checkbox-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    color: var(--text-primary);
  }

  .file-color-indicator {
    width: 1rem;
    height: 1rem;
    border-radius: 0.25rem;
  }

  .file-selection-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .action-btn {
    padding: 0.375rem 0.75rem;
    background: var(--accent-primary);
    color: white;
    border: none;
    border-radius: 0.375rem;
    font-size: 0.8rem;
    font-weight: 500;
    cursor: pointer;
  }

  .selected-count {
    margin-left: auto;
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  .legend {
    margin-bottom: 2rem;
    padding: 1rem;
    background: var(--bg-secondary);
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
  }

  .legend-header {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }

  .legend h3 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  .legend-items {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    max-height: 10rem;
    overflow-y: auto;
    padding-right: 0.25rem;
    /* Subtle scrollbar so it doesn't look broken when small */
    scrollbar-width: thin;
    scrollbar-color: var(--border-color-dark) transparent;
  }

  .legend-items::-webkit-scrollbar { width: 5px; }
  .legend-items::-webkit-scrollbar-track { background: transparent; }
  .legend-items::-webkit-scrollbar-thumb { background: var(--border-color-dark); border-radius: 3px; }
  .legend-item { display: flex; align-items: center; gap: 0.5rem; }
  .legend-color { width: 1.5rem; height: 0.75rem; border-radius: 0.125rem; }
  .legend-item span { font-size: 0.8rem; color: var(--text-secondary); }

  .window-info {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
    padding: 1rem;
    background: var(--bg-accent);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
  }

  .window-position {
    color: var(--text-primary);
    font-size: 0.875rem;
  }

  .window-count {
    margin-left: 1rem;
    color: var(--text-secondary);
    cursor: pointer;
    user-select: none;
    padding: 0.25rem 0.5rem;
    border-radius: 0.25rem;
  }

  .window-page-input {
    width: 5rem;
    padding: 0.25rem 0.5rem;
    margin-left: 0.5rem;
    text-align: center;
    font-size: 0.875rem;
    background: var(--bg-primary);
    border: 2px solid var(--accent-primary);
    border-radius: 0.25rem;
    color: var(--text-primary);
  }

  .search-bar {
    position: relative;
    max-width: 100%;
    width: 100%;
    box-sizing: border-box;
  }

  .search-input {
    width: 100%;
    padding: 0.5rem 2.5rem 0.5rem 0.75rem;
    font-size: 0.8rem;
    border: 1px solid var(--border-color-dark);
    border-radius: 0.375rem;
    background: var(--bg-primary);
    color: var(--text-primary);
    box-sizing: border-box;
  }
  .search-input:focus { outline: none; border-color: var(--accent-primary); }
  .search-input::placeholder { color: var(--text-tertiary); }
  .search-indicator {
    margin-left: 0.5rem;
    font-size: 0.8rem;
    color: var(--accent-primary);
    font-weight: 500;
  }

  .filtered-pages {
    margin-left: 0.25rem;
    font-size: 0.7rem;
    color: var(--accent-primary);
    font-style: italic;
  }

  .window-navigation { display: flex; gap: 0.5rem; }
  .window-navigation button {
    padding: 0.5rem 1rem;
    border: none;
    border-radius: 0.375rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
  }

  .window-navigation button:not(:disabled) {
    background: var(--accent-primary);
    color: white;
  }

  .window-navigation button:disabled {
    background: var(--bg-hover);
    color: var(--text-tertiary);
    cursor: not-allowed;
  }

  .browser {
    position: relative;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
    min-height: 400px;
    display: flex;
    flex-direction: column;
  }
  .browser-inner { position: relative; padding: 0 2rem; }

  .empty-state {
    text-align: center;
    padding: 4rem;
    color: var(--text-secondary);
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .position-markers {
    position: sticky;
    top: 0;
    height: 28px;
    margin-bottom: 2rem;
    background: var(--bg-secondary);
    border-bottom: 2px solid var(--border-color-dark);
    flex-shrink: 0;
    z-index: 20;
    pointer-events: none;
  }

  .marker {
    position: absolute;
    font-size: 0.7rem;
    color: var(--text-secondary);
  }

  .marker-tick {
    width: 1px;
    height: 10px;
    background: var(--border-color-dark);
    margin-top: 1.5rem;
  }

  .marker-label {
    white-space: nowrap;
    margin-left: -1rem;
  }

  .contigs-viewport {
    flex: 1;
    overflow-y: auto;
    padding-bottom: 1rem;
    max-height: 500px;
  }

  .contigs-container {
    display: flex;
    flex-direction: column;
  }

  .contig-track {
    position: relative;
    height: 8px;
    margin-bottom: 0.75rem;
    min-height: 8px;
  }

  .contig {
    position: absolute;
    height: 8px;
    top: 5px;
    cursor: pointer;
    border: 1px solid rgba(0, 0, 0, 0.2);
    border-radius: 3px;
    box-sizing: border-box;
  }

  .contig.hovered {
    border: 2px solid white;
    z-index: 15;
  }

  .tooltip {
    position: fixed;
    bottom: 2rem;
    left: 50%;
    transform: translateX(-50%);
    padding: 1rem;
    background: var(--bg-primary);
    border: 2px solid var(--border-color-dark);
    border-radius: 0.5rem;
    box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
    z-index: 1000;
    min-width: 320px;
    max-width: 400px;
  }

  .tooltip-header {
    margin-bottom: 0.5rem;
    font-weight: 600;
    color: var(--accent-primary);
    font-size: 0.95rem;
  }

  .tooltip-body {
    display: flex;
    gap: 0.5rem;
    flex-direction: column;
  }

  .tooltip-file {
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--border-color);
  }

  .file-badge {
    padding: 0.25rem 0.5rem;
    border: 1px solid;
    border-radius: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    white-space: nowrap;
  }

  .tooltip-content {
    font-size: 0.8rem;
    color: var(--text-secondary);
    display: grid;
    gap: 0.25rem;
  }

  .tooltip-content strong {
    color: var(--text-primary);
  }

  .overview-panel {
    margin-bottom: 1rem;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
    overflow: hidden;
  }

  .overview-toggle {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    width: 100%;
    padding: 0.75rem 1rem;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-primary);
    font-size: 0.875rem;
    font-weight: 600;
    text-align: left;
  }
  .overview-toggle:hover { background: var(--bg-hover); }

  .toggle-chevron {
    flex-shrink: 0;
    transition: transform 0.2s ease;
    color: var(--text-secondary);
  }

  .toggle-chevron.rotated { transform: rotate(90deg); }
  .overview-toggle-title { flex: 1; }

  .overview-search-badge {
    padding: 0.15rem 0.5rem;
    background: var(--accent-light);
    color: var(--accent-primary);
    border-radius: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
  }

  .overview-body {
    padding: 0 1rem 1rem;
    padding-top: 0.5rem;
    padding-bottom: 1.5rem;
    display: flex;
    gap: 1.25rem;
    flex-direction: column;
    animation: fadeIn 0.15s ease-in;
  }

  .overview-empty {
    color: var(--text-secondary);
    font-size: 0.8rem;
    text-align: center;
    padding: 1rem 0;
  }

  .overview-genome {
    display: flex;
    gap: 0.25rem;
    flex-direction: column;
  }

  .overview-genome-header {
    display: flex;
    gap: 0.4rem;
    align-items: center;
    margin-bottom: 0.25rem;
  }

  .overview-genome-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .overview-genome-name {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  .overview-lines {
    display: flex;
    gap: 4px;
    flex-direction: column;
  }

  .overview-chr-row {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    height: 32px;
  }

  .overview-chr-label {
    width: 42px;
    flex-shrink: 0;
    font-size: 0.65rem;
    color: var(--text-tertiary);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .overview-chr-track {
    position: relative;
    flex: 1;
    height: 18px;
    display: flex;
    align-items: center;
    margin-right: 2rem;
  }

  .overview-line-bg {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 2px;
    background: var(--border-color);
    transform: translateY(-50%);
  }

  .overview-marker {
    position: absolute;
    top: 50%;
    width: 1px;
    height: 8px;
    background: var(--border-color);
    transform: translate(-50%, -50%);
  }

  .overview-marker-end {
    height: 12px;
    background: var(--text-secondary);
  }

  .overview-marker-label {
    position: absolute;
    top: 9px;
    left: 50%;
    transform: translateX(-50%);
    font-size: 0.65rem;
    opacity: 0.85;
    color: var(--text-secondary);
    white-space: nowrap;
    pointer-events: none;
  }

  .overview-marker-label.overview-marker-label-end {
    font-size: 0.7rem;
    opacity: 1;
    font-weight: 600;
  }

  .overview-dot {
    position: absolute;
    top: 50%;
    width: 8px;
    height: 8px;
    /* Green to signal "hit found here". Distinct from the accent
       colour used for the chromosome markers themselves so the user
       can tell at a glance which dots are markers vs search hits. */
    background: #22c55e;
    border: 1px solid var(--bg-secondary);
    border-radius: 50%;
    transform: translate(-50%, -50%);
    cursor: pointer;
    padding: 0;
    z-index: 2;
  }

  .overview-dot-tooltip {
    display: none;
    position: absolute;
    bottom: 14px;
    left: 50%;
    transform: translateX(-50%);
    padding: 0.3rem 0.5rem;
    background: var(--bg-primary);
    color: var(--text-primary);
    border: 1px solid var(--border-color-dark);
    border-radius: 0.25rem;
    font-size: 0.7rem;
    line-height: 1.4;
    white-space: nowrap;
    pointer-events: none;
    text-align: center;
  }

  .overview-dot:hover .overview-dot-tooltip {
    display: block;
  }

  .comparison-panel {
    margin-bottom: 1rem;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 0.5rem;
    overflow: hidden;
  }

  .comparison-count-badge {
    padding: 0.15rem 0.5rem;
    background: var(--bg-hover);
    color: var(--text-secondary);
    border-radius: 0.25rem;
    font-size: 0.7rem;
    font-weight: 500;
  }

  .comparison-body {
    padding: 0 1rem 0.5rem;
    max-height: 20rem;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--border-color-dark) transparent;
    animation: fadeIn 0.15s ease-in;
  }
  .comparison-body::-webkit-scrollbar { width: 5px; }
  .comparison-body::-webkit-scrollbar-track { background: transparent;}
  .comparison-body::-webkit-scrollbar-thumb { background: var(--border-color-dark); border-radius: 3px; }
  .comparison-section { padding: 0.5rem 0; }
  .comparison-section + .comparison-section {
    border-top: 1px solid var(--border-color);
    padding-top: 0.75rem;
  }
  .comparison-section-header {
    display: flex;
    gap: 0.4rem;
    align-items: center;
    margin-bottom: 0.4rem;
  }
  .comparison-genome-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .comparison-section-title {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text-primary);
    flex: 1;
  }

  .comparison-section-count {
    font-size: 0.7rem;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .comparison-none {
    font-size: 0.75rem;
    color: var(--text-tertiary);
    font-style: italic;
    margin: 0;
  }

  @media (max-width: 768px) {
    .analysis-container {
      padding: 1rem;
    }

    .window-info {
      flex-direction: column;
      gap: 1rem;
    }

    .file-selection {
      flex-direction: column;
    }

    .position-markers { padding: 0 1rem; }

    .contigs-viewport { padding: 1rem; }

    .tooltip {
      left: 1rem;
      right: 1rem;
      transform: none;
      max-width: calc(100% - 2rem);
    }
  }

  .action-btn:hover {
    filter: brightness(0.88);
    transition: filter 0.15s ease;
  }

  .window-navigation button:not(:disabled):hover {
    filter: brightness(0.88);
    transition: filter 0.15s ease;
  }
</style>