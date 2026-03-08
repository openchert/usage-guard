export interface UsageRingTheme {
  accentColor?: string;
  size?: number;
  thickness?: number;
  trackColor?: string;
  labelColor?: string;
  labelSize?: string;
  labelWeight?: number | string;
  innerFill?: string;
}

export interface ResolvedUsageRingTheme {
  accentColor: string;
  size: number;
  thickness: number;
  trackColor: string;
  labelColor: string;
  labelSize: string;
  labelWeight: number | string;
  innerFill: string;
}

export const DEFAULT_USAGE_RING_THEME: ResolvedUsageRingTheme = {
  accentColor: '#5a6680',
  size: 32,
  thickness: 5,
  trackColor: 'rgba(255, 255, 255, 0.08)',
  labelColor: 'rgba(229, 232, 242, 0.96)',
  labelSize: '8px',
  labelWeight: 600,
  innerFill: 'transparent',
};

export function resolveUsageRingTheme(
  theme?: UsageRingTheme,
  fallbackAccent = DEFAULT_USAGE_RING_THEME.accentColor,
): ResolvedUsageRingTheme {
  return {
    ...DEFAULT_USAGE_RING_THEME,
    ...theme,
    accentColor: theme?.accentColor ?? fallbackAccent,
  };
}
