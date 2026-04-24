import { render } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import DonutVisualisation from '../src/routes/DonutVisualisation.svelte';
import { mockBackendMatch } from './__mocks__/bincodeMock';

describe('DonutVisualisation', () => {
  it('renders chart when given matches', () => {
    const { container } = render(DonutVisualisation, {
      props: {
        matches: [mockBackendMatch],
        files: [
          { name: 'test1.xmap', rows: 0, color: '#3b82f6' },
          { name: 'test2.xmap', rows: 0, color: '#10b981' },
        ],
      },
    });
    const svg = container.querySelector('svg');
    expect(svg).toBeTruthy();
  });
});
