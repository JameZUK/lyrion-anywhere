// Listening stats (docs/plans/wave3-design.md §1): the periods offered, the device's time
// zone as the gateway wants it, and the words and numbers the page shows.
import { STATS_LIMITS, statsDate } from '#lib/api/me';

export interface PeriodChoice {
	value: string;
	label: string;
}

/**
 * The device's offset in minutes east of UTC, as the gateway wants it. JavaScript's
 * getTimezoneOffset counts minutes west (UTC+1 gives −60), so it is turned round, and kept
 * within the contract's −840 to 840.
 */
export function deviceTz(d: Date = new Date()): number {
	const west = d.getTimezoneOffset();
	if (!Number.isFinite(west)) return 0;
	const east = Math.round(-west);
	return Math.max(-STATS_LIMITS.tzMax, Math.min(STATS_LIMITS.tzMax, east)) || 0;
}

const pad = (n: number) => String(n).padStart(2, '0');

/** This month, as a period (the device's own calendar). */
export function monthPeriod(d: Date = new Date()): string {
	return `month-${d.getFullYear()}-${pad(d.getMonth() + 1)}`;
}

export function yearPeriod(y: number): string {
	return `year-${y}`;
}

/** The year a period covers, for `year-YYYY` only. */
export function periodYear(p: string): number | null {
	const m = p.match(/^year-(\d{4})$/);
	return m ? Number(m[1]) : null;
}

/**
 * The years the history reaches into, newest first: this year, and each earlier year that
 * `retention` days back from `now` still touches.
 */
export function historyYears(now: Date, retentionDays: number): number[] {
	const days = Math.max(0, Math.min(36500, Math.floor(Number(retentionDays) || 0)));
	const oldest = new Date(now.getTime() - days * 86400_000).getFullYear();
	const out: number[] = [];
	for (let y = now.getFullYear(); y >= oldest && out.length < 100; y--) out.push(y);
	return out;
}

/** The periods offered: this month, the last 30 days, this year, past years, everything kept. */
export function periodChoices(now: Date, retentionDays: number): PeriodChoice[] {
	const year = now.getFullYear();
	const past = historyYears(now, retentionDays).filter((y) => y < year);
	return [
		{ value: monthPeriod(now), label: 'This month' },
		{ value: 'days-30', label: 'Last 30 days' },
		{ value: yearPeriod(year), label: 'This year' },
		...past.map((y) => ({ value: yearPeriod(y), label: String(y) })),
		{ value: 'all', label: 'Everything kept' },
	];
}

/** How a period reads in a sentence: "this month", "2025", "the last 30 days"… */
export function periodWords(p: string, now: Date = new Date()): string {
	if (p === 'all') return 'in your history';
	if (p === monthPeriod(now)) return 'this month';
	const y = periodYear(p);
	if (y !== null) return y === now.getFullYear() ? 'this year' : `in ${y}`;
	const d = p.match(/^days-(\d+)$/);
	if (d) return `in the last ${d[1]} days`;
	const m = p.match(/^month-(\d{4})-(\d{2})$/);
	if (m) return `in ${monthName(`${m[1]}-${m[2]}`)}`;
	return '';
}

/**
 * Whether "new" artists may just be old favourites from before the history begins: for
 * everything kept and for long periods (wave3-design.md §1, New artists).
 */
export function newMeansSince(p: string): boolean {
	if (p === 'all' || p.startsWith('year-')) return true;
	const d = p.match(/^days-(\d+)$/);
	return !!d && Number(d[1]) > 60;
}

/**
 * When the "Your year in sound" story is offered unasked: December for this year, and
 * January for the year just ended. Null the rest of the year (it is still there on demand).
 */
export function storyYear(now: Date = new Date()): number | null {
	const m = now.getMonth();
	if (m === 11) return now.getFullYear();
	if (m === 0) return now.getFullYear() - 1;
	return null;
}

/** The story needs a week of listening days; fewer, and the page says how many to go. */
export const STORY_MIN_DAYS = 7;
export function storyDaysToGo(days: number): number {
	return Math.max(0, STORY_MIN_DAYS - Math.max(0, Math.floor(days)));
}

// --- words and numbers ---------------------------------------------------------------------

const nf = new Intl.NumberFormat('en-GB');
export const number = (n: number) => nf.format(n);

/** "23 h", "1.5 h" (under 10 hours), "45 min", "0 min". */
export function duration(seconds: number): string {
	const s = Math.max(0, Number(seconds) || 0);
	if (s < 3600) return `${Math.round(s / 60)} min`;
	const h = s / 3600;
	return h < 10 ? `${nf.format(Math.round(h * 10) / 10)} h` : `${nf.format(Math.round(h))} h`;
}

/** The same in words for screen readers and sentences: "23 hours", "45 minutes". */
export function durationWords(seconds: number): string {
	const s = Math.max(0, Number(seconds) || 0);
	if (s < 3600) {
		const m = Math.round(s / 60);
		return `${m} ${m === 1 ? 'minute' : 'minutes'}`;
	}
	const h = s / 3600;
	const v = h < 10 ? Math.round(h * 10) / 10 : Math.round(h);
	return `${nf.format(v)} ${v === 1 ? 'hour' : 'hours'}`;
}

export const plays = (n: number) => `${nf.format(n)} ${n === 1 ? 'play' : 'plays'}`;

export const WEEKDAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
export const WEEKDAYS_SHORT = ['M', 'T', 'W', 'T', 'F', 'S', 'S'];
const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

/** "21:00" for hour 21. */
export const hourName = (h: number) => `${pad(h)}:00`;
/** "21:00–22:00". */
export const hourRange = (h: number) => `${pad(h)}:00–${pad((h + 1) % 24)}:00`;

/** "October 2026" for "2026-10" ('' if it isn't one). */
export function monthName(ym: string): string {
	const m = ym.match(/^(\d{4})-(0[1-9]|1[0-2])$/);
	return m ? `${MONTHS[Number(m[2]) - 1]} ${m[1]}` : '';
}

/** "4 October 2026" for "2026-10-04" ('' if it isn't a date). Read as written: no time zone. */
export function dayName(date: string): string {
	const d = statsDate(date);
	if (!d) return '';
	const [y, m, day] = d.split('-').map(Number) as [number, number, number];
	return `${day} ${MONTHS[m - 1]} ${y}`;
}

/** The first index of the largest value, or −1 when every value is 0. */
export function peak(values: readonly number[]): number {
	let best = -1;
	for (let i = 0; i < values.length; i++) if ((values[i] ?? 0) > 0 && (best < 0 || values[i]! > values[best]!)) best = i;
	return best;
}

// --- chart summaries (the charts' text alternatives) ----------------------------------------

export function hourSummary(byHour: readonly number[]): string {
	const i = peak(byHour);
	if (i < 0) return 'No plays to show by hour.';
	return `You listen most between ${hourRange(i)}, with ${plays(byHour[i]!)}.`;
}

export function weekdaySummary(byWeekday: readonly number[]): string {
	const i = peak(byWeekday);
	if (i < 0) return 'No plays to show by weekday.';
	return `Your busiest day of the week is ${WEEKDAYS[i]}, with ${plays(byWeekday[i]!)}.`;
}

export function monthSummary(byMonth: readonly { month: string; seconds: number; plays: number }[]): string {
	const i = peak(byMonth.map((m) => m.seconds));
	if (i < 0) return 'No listening to show by month.';
	const m = byMonth[i]!;
	return `Your biggest month was ${monthName(m.month)}, with ${durationWords(m.seconds)} and ${plays(m.plays)}.`;
}
