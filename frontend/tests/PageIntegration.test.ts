import { render } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import Page from "../src/routes/+page.svelte";
import { mockBackendMatch } from './__mocks__/bincodeMock';

describe('Integration: +page.svelte', () => {
  it('renders donut visualisation when matches loaded', async () => {
    const { container } = render(Page, {
      props: {
        matches: [mockBackendMatch],
        files: [
          { name: 'genome1.xmap', rows: 100, color: '#3b82f6' },
          { name: 'genome2.xmap', rows: 200, color: '#10b981' },
        ],
      },
    });

    const charts = container.querySelectorAll('svg');
    expect(charts.length).toBeGreaterThan(0);
  });
});
