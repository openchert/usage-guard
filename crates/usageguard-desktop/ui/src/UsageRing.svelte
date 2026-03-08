<script lang="ts">
  import { resolveUsageRingTheme } from './usageRingTheme';
  import type { UsageRingTheme } from './usageRingTheme';

  export let ratio = 0;
  export let accent = '#5a6680';
  export let label = '';
  export let theme: UsageRingTheme | undefined = undefined;

  function normalizeRatio(value: number): number {
    if (!Number.isFinite(value)) return 0;
    return Math.min(1, Math.max(0, value));
  }

  $: resolvedTheme = resolveUsageRingTheme(theme, accent);
  $: normalizedRatio = normalizeRatio(ratio);
  $: radius = 50 - resolvedTheme.thickness / 2;
  $: innerRadius = Math.max(0, 50 - resolvedTheme.thickness);
  $: progressLength = normalizedRatio * 100;
  $: cssVars = [
    `--ring-size:${resolvedTheme.size}px`,
    `--ring-label-color:${resolvedTheme.labelColor}`,
    `--ring-label-size:${resolvedTheme.labelSize}`,
    `--ring-label-weight:${resolvedTheme.labelWeight}`,
  ].join(';');
</script>

<div class="usage-ring" style={cssVars}>
  <svg class="ring-svg" viewBox="0 0 100 100" aria-hidden="true">
    {#if resolvedTheme.innerFill !== 'transparent'}
      <circle cx="50" cy="50" r={innerRadius} fill={resolvedTheme.innerFill} />
    {/if}

    <circle
      class="ring-track"
      cx="50"
      cy="50"
      r={radius}
      fill="none"
      stroke={resolvedTheme.trackColor}
      stroke-width={resolvedTheme.thickness}
    />

    <circle
      class="ring-progress"
      cx="50"
      cy="50"
      r={radius}
      fill="none"
      stroke={resolvedTheme.accentColor}
      stroke-width={resolvedTheme.thickness}
      stroke-linecap={normalizedRatio > 0 ? 'round' : 'butt'}
      stroke-dasharray={`${progressLength} 100`}
      pathLength="100"
    />
  </svg>

  {#if label}
    <span class="ring-label">{label}</span>
  {/if}
</div>

<style>
  .usage-ring {
    position: relative;
    display: grid;
    place-items: center;
    width: var(--ring-size);
    height: var(--ring-size);
    border-radius: 50%;
  }

  .ring-svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
  }

  .ring-track,
  .ring-progress {
    shape-rendering: geometricPrecision;
    vector-effect: non-scaling-stroke;
  }

  .ring-progress {
    transform: rotate(-90deg);
    transform-origin: 50px 50px;
  }

  .ring-label {
    position: relative;
    z-index: 1;
    font-size: var(--ring-label-size);
    font-weight: var(--ring-label-weight);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--ring-label-color);
  }
</style>
