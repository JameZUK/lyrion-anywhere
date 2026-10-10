// Shapes of the LMS responses the app uses. Every field is optional: LMS omits fields freely,
// and nothing here is trusted beyond being displayed as text.

export interface PlayerInfo {
	playerid: string;
	name: string;
	model?: string;
	modelname?: string;
	connected?: number;
	power?: number;
	isplaying?: number;
}
export interface PlayersResult {
	count?: number;
	players_loop?: PlayerInfo[];
}

export interface Track {
	id?: number;
	title?: string;
	artist?: string;
	album?: string;
	album_id?: number;
	artist_id?: number | string;
	duration?: number;
	tracknum?: number | string;
	coverid?: string;
	artwork_track_id?: string;
	artwork_url?: string;
	year?: number | string;
	type?: string;
	bitrate?: string;
	samplerate?: string;
	samplesize?: string;
	remote?: number;
	remote_title?: string;
	'playlist index'?: number;
}

export interface PlayerStatus {
	player_name?: string;
	player_connected?: number;
	power?: number;
	mode?: 'play' | 'pause' | 'stop';
	time?: number;
	duration?: number;
	'mixer volume'?: number;
	playlist_cur_index?: number | string;
	playlist_tracks?: number;
	'playlist shuffle'?: number;
	'playlist repeat'?: number;
	can_seek?: number;
	remote?: number;
	current_title?: string;
	playlist_loop?: Track[];
}

export interface Album {
	id: number;
	album?: string;
	artist?: string;
	artwork_track_id?: string;
	year?: number | string;
	artist_id?: number | string;
}
export interface AlbumsResult {
	count?: number;
	albums_loop?: Album[];
}
export interface Artist {
	id: number;
	artist?: string;
}
export interface ArtistsResult {
	count?: number;
	artists_loop?: Artist[];
}
export interface TitlesResult {
	count?: number;
	titles_loop?: Track[];
}
export interface SearchResult {
	count?: number;
	contributors_loop?: { contributor?: string; contributor_id?: number }[];
	albums_loop?: { album?: string; album_id?: number }[];
	tracks_loop?: { track?: string; track_id?: number }[];
}

export interface MenuItem {
	id?: string;
	name?: string;
	type?: string;
	image?: string;
	isaudio?: number | string;
	hasitems?: number | string;
}
export interface FavouritesResult {
	count?: number;
	title?: string;
	loop_loop?: MenuItem[];
}
export interface Playlist {
	id: number;
	playlist?: string;
}
export interface PlaylistsResult {
	count?: number;
	playlists_loop?: Playlist[];
}
export interface PlaylistTracksResult {
	count?: number;
	playlisttracks_loop?: Track[];
}
