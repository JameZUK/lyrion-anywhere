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
	remote?: number | string;
	remote_title?: string;
	/** File size in bytes (tag `f`). */
	filesize?: number | string;
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
	remote?: number | string;
	current_title?: string;
	playlist_loop?: Track[];
	/** Seconds left on the sleep timer, while one is set. */
	will_sleep_in?: number | string;
	/** The sleep timer's length, in seconds, while one is set. */
	sleep?: number | string;
	/** Set while the player is in a sync group (only players the caller may control). */
	sync_master?: string;
	sync_slaves?: string;
}

export interface SyncGroup {
	sync_members?: string;
	sync_member_names?: string;
}
export interface SyncGroupsResult {
	syncgroups_loop?: SyncGroup[];
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

export interface Genre {
	id: number;
	genre?: string;
}
export interface GenresResult {
	count?: number;
	genres_loop?: Genre[];
}
export interface YearsResult {
	count?: number;
	years_loop?: { year?: number | string }[];
}
/** A music folder entry. LMS's `url` (the file's path) never reaches the app. */
export interface FolderItem {
	id: number;
	filename?: string;
	title?: string;
	type?: string;
	coverid?: string;
	duration?: string;
}
export interface MusicFolderResult {
	count?: number;
	folder_loop?: FolderItem[];
}
/** LMS virtual libraries (`libraries`). */
export interface LibrariesResult {
	folder_loop?: { id?: string; name?: string }[];
}

export interface AppInfo {
	cmd?: string;
	name?: string;
	icon?: string;
	type?: string;
	weight?: number;
}
export interface AppsResult {
	count?: number;
	appss_loop?: AppInfo[];
	radioss_loop?: AppInfo[];
}
