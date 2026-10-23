// "Your year in sound" (wave3-design.md §1): the cards of the story, from a year's stats.
// Every card is plain text, so the same card can be shown, read out and drawn to an image.
import type { ListeningStats } from '#lib/api/me';
import { textOf } from '#lib/lms/format';
import type { StatsNames } from './names';
import { dayName, durationWords, hourRange, number, peak, plays } from './period';

export interface StoryCard {
	/** Stable key: hours, artist, album, track, five, discovery, hour, streak, summary. */
	key: string;
	/** The small line above ("Your top artist"). */
	kicker: string;
	/** The big line ("An Example Band"). */
	headline: string;
	/** The line under it ("40 plays, 2.5 hours"). */
	detail: string;
	/** A numbered list (the top five). */
	list?: string[];
}

const artistName = (n: string) => n || 'An unnamed artist';

/** The cards for `year`; cards with nothing to say are left out. */
export function storyCards(year: number, s: ListeningStats, names: StatsNames): StoryCard[] {
	const cards: StoryCard[] = [];
	const t = s.totals;
	cards.push({
		key: 'hours',
		kicker: `Your ${year} in sound`,
		headline: durationWords(t.seconds),
		detail: `of music: ${plays(t.plays)} of ${number(t.tracks)} different ${t.tracks === 1 ? 'track' : 'tracks'}.`,
	});
	const artist = s.top_artists[0];
	if (artist) {
		cards.push({ key: 'artist', kicker: 'Your top artist', headline: artistName(artist.name), detail: `${plays(artist.plays)}, ${durationWords(artist.seconds)}.` });
	}
	const album = s.top_albums[0];
	if (album) {
		const a = names.albums.get(album.id);
		const by = a ? textOf(a.artist) : '';
		cards.push({
			key: 'album',
			kicker: 'Your top album',
			headline: a ? textOf(a.album) || 'An untitled album' : 'An album no longer in the library',
			detail: `${by ? `${by} · ` : ''}${plays(album.plays)}.`,
		});
	}
	const track = s.top_tracks[0];
	if (track) {
		const tr = names.tracks.get(track.id);
		const by = tr ? textOf(tr.artist) : '';
		cards.push({
			key: 'track',
			kicker: 'Your top track',
			headline: tr ? textOf(tr.title) || 'An untitled track' : 'A track no longer in the library',
			detail: `${by ? `${by} · ` : ''}${plays(track.plays)}.`,
		});
	}
	if (s.top_artists.length >= 2) {
		cards.push({
			key: 'five',
			kicker: 'Your top five artists',
			headline: `${Math.min(5, s.top_artists.length)} artists you kept coming back to`,
			detail: `${number(t.artists)} ${t.artists === 1 ? 'artist' : 'artists'} in all.`,
			list: s.top_artists.slice(0, 5).map((a) => artistName(a.name)),
		});
	}
	const found = s.new_artists[0];
	if (found) {
		cards.push({ key: 'discovery', kicker: 'A new discovery', headline: artistName(found.name), detail: `First played this year${found.first_played ? `, on ${firstPlayed(found.first_played)}` : ''}.` });
	}
	const h = peak(s.by_hour);
	if (h >= 0) cards.push({ key: 'hour', kicker: 'Your busiest hour', headline: hourRange(h), detail: `${plays(s.by_hour[h]!)} started then.` });
	if (s.longest_streak) {
		const k = s.longest_streak;
		cards.push({
			key: 'streak',
			kicker: 'Your longest streak',
			headline: `${number(k.days)} ${k.days === 1 ? 'day' : 'days'} in a row`,
			detail: `From ${dayName(k.from)} to ${dayName(k.to)}.`,
		});
	}
	cards.push({
		key: 'summary',
		kicker: `That was ${year}`,
		headline: `${durationWords(t.seconds)}, ${plays(t.plays)}`,
		detail: [
			`${number(t.artists)} ${t.artists === 1 ? 'artist' : 'artists'}, ${number(t.albums)} ${t.albums === 1 ? 'album' : 'albums'}`,
			artist ? `top artist ${artistName(artist.name)}` : '',
			`music on ${number(t.days)} ${t.days === 1 ? 'day' : 'days'}`,
		]
			.filter(Boolean)
			.join(' · '),
	});
	return cards;
}

function firstPlayed(unix: number): string {
	const d = new Date(unix * 1000);
	return d.toLocaleDateString('en-GB', { day: 'numeric', month: 'long' });
}
