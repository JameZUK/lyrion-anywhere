// Listening stats: the periods offered, the time zone sent, and the words shown.
import * as P from '#lib/stats/period';
import { statsPeriod } from '#lib/api/me';

/** A date whose offset is fixed, whatever the machine running the tests uses. */
function at(iso: string, westMinutes: number): Date {
	const d = new Date(iso);
	d.getTimezoneOffset = () => westMinutes;
	return d;
}

test('tz: JavaScript counts minutes west, the gateway wants minutes east', () => {
	expect(P.deviceTz(at('2026-10-13T12:00:00Z', -60))).toBe(60); // UTC+1 (British Summer Time)
	expect(P.deviceTz(at('2026-10-13T12:00:00Z', 300))).toBe(-300); // UTC−5
	expect(P.deviceTz(at('2026-10-13T12:00:00Z', -330))).toBe(330); // UTC+5:30
	expect(P.deviceTz(at('2026-10-13T12:00:00Z', -345))).toBe(345); // UTC+5:45
	expect(P.deviceTz(at('2026-10-13T12:00:00Z', -840))).toBe(840); // UTC+14, the far edge
	expect(P.deviceTz(at('2026-10-13T12:00:00Z', 720))).toBe(-720); // UTC−12
	expect(Object.is(P.deviceTz(at('2026-10-13T12:00:00Z', 0)), 0)).toBe(true); // never −0
	// Anything out of the contract's range is clamped; nonsense becomes 0.
	expect(P.deviceTz(at('2026-10-13T12:00:00Z', -900))).toBe(840);
	expect(P.deviceTz(at('2026-10-13T12:00:00Z', 900))).toBe(-840);
	expect(P.deviceTz(at('2026-10-13T12:00:00Z', NaN))).toBe(0);
});

test('this month and the year follow the device’s own calendar', () => {
	expect(P.monthPeriod(new Date(2026, 9, 13))).toBe('month-2026-10');
	expect(P.monthPeriod(new Date(2026, 0, 1, 0, 0))).toBe('month-2026-01');
	expect(P.monthPeriod(new Date(2025, 11, 31, 23, 59))).toBe('month-2025-12');
	expect(P.yearPeriod(2026)).toBe('year-2026');
	expect(P.periodYear('year-2025')).toBe(2025);
	expect(P.periodYear('month-2025-01')).toBeNull();
});

test('periods offered: this month, last 30 days, this year, past years the history reaches, everything', () => {
	const now = new Date(2026, 9, 13);
	expect(P.periodChoices(now, 396)).toEqual([
		{ value: 'month-2026-10', label: 'This month' },
		{ value: 'days-30', label: 'Last 30 days' },
		{ value: 'year-2026', label: 'This year' },
		{ value: 'year-2025', label: '2025' },
		{ value: 'all', label: 'Everything kept' },
	]);
	// 13 months back from mid-January reaches into the year before last.
	expect(P.historyYears(new Date(2026, 0, 15), 396)).toEqual([2026, 2025, 2024]);
	expect(P.historyYears(now, 0)).toEqual([2026]);
	expect(P.historyYears(now, -5)).toEqual([2026]);
	expect(P.historyYears(now, 1e9).length).toBeLessThanOrEqual(100);
	// Every value offered is one the API accepts.
	for (const c of P.periodChoices(now, 3650)) expect(() => statsPeriod(c.value)).not.toThrow();
});

test('the story is offered from 1 December to 31 January, for the year', () => {
	expect(P.storyYear(new Date(2026, 10, 30, 23, 59))).toBeNull();
	expect(P.storyYear(new Date(2026, 11, 1, 0, 0))).toBe(2026);
	expect(P.storyYear(new Date(2026, 11, 31, 23, 59))).toBe(2026);
	expect(P.storyYear(new Date(2027, 0, 1))).toBe(2026);
	expect(P.storyYear(new Date(2027, 0, 31, 23, 59))).toBe(2026);
	expect(P.storyYear(new Date(2027, 1, 1))).toBeNull();
	expect(P.storyDaysToGo(0)).toBe(7);
	expect(P.storyDaysToGo(5)).toBe(2);
	expect(P.storyDaysToGo(30)).toBe(0);
});

test('words: periods, durations and dates in British English', () => {
	const now = new Date(2026, 9, 13);
	expect(P.periodWords('month-2026-10', now)).toBe('this month');
	expect(P.periodWords('month-2026-09', now)).toBe('in September 2026');
	expect(P.periodWords('year-2026', now)).toBe('this year');
	expect(P.periodWords('year-2025', now)).toBe('in 2025');
	expect(P.periodWords('days-30', now)).toBe('in the last 30 days');
	expect(P.periodWords('all', now)).toBe('in your history');
	expect(P.duration(82800)).toBe('23 h');
	expect(P.duration(5400)).toBe('1.5 h');
	expect(P.duration(2700)).toBe('45 min');
	expect(P.duration(-5)).toBe('0 min');
	expect(P.duration(3600 * 12345)).toBe('12,345 h');
	expect(P.durationWords(3600)).toBe('1 hour');
	expect(P.durationWords(60)).toBe('1 minute');
	expect(P.plays(1)).toBe('1 play');
	expect(P.plays(1200)).toBe('1,200 plays');
	expect(P.dayName('2026-10-04')).toBe('4 October 2026');
	expect(P.dayName('2026-02-30')).toBe('');
	expect(P.monthName('2026-10')).toBe('October 2026');
	expect(P.hourRange(23)).toBe('23:00–00:00');
	expect(P.newMeansSince('all')).toBe(true);
	expect(P.newMeansSince('year-2026')).toBe(true);
	expect(P.newMeansSince('days-90')).toBe(true);
	expect(P.newMeansSince('days-30')).toBe(false);
	expect(P.newMeansSince('month-2026-10')).toBe(false);
});

test('chart summaries name the peak, or say there is nothing', () => {
	const hours = Array(24).fill(0);
	hours[21] = 40;
	hours[8] = 40; // a tie: the earlier hour
	expect(P.hourSummary(hours)).toBe('You listen most between 08:00–09:00, with 40 plays.');
	expect(P.hourSummary(Array(24).fill(0))).toBe('No plays to show by hour.');
	expect(P.weekdaySummary([1, 2, 3, 4, 5, 30, 2])).toBe('Your busiest day of the week is Saturday, with 30 plays.');
	expect(P.monthSummary([{ month: '2026-10', plays: 412, seconds: 82800 }])).toBe('Your biggest month was October 2026, with 23 hours and 412 plays.');
	expect(P.monthSummary([])).toBe('No listening to show by month.');
});
