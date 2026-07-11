/** Settings sidebar sections — shared by Settings page and legacy route redirects. */
const SETTINGS_SECTIONS = [
  'general',
  'appearance',
  'integrations',
  'agents',
  'notifications',
  'usage',
  'rubrics',
  'ops',
  'memories',
  'about',
] as const;

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

export function isSettingsSection(value: string | null | undefined): value is SettingsSection {
  return SETTINGS_SECTIONS.includes(value as SettingsSection);
}
