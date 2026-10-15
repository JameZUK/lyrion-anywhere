package io.github.jamezuk.lyrionanywhere.ui.player

import io.github.jamezuk.lyrionanywhere.data.offline.Reachability

/**
 * One choice in *Sync with…* (Play on…): a player on its own, or a whole sync group as one
 * entry. The controlled player joins it through [playerId], the group's first member as LMS
 * lists it (`<playerId> sync <controlled>`: the argument joins the subject's group).
 * [memberIds] are the group's members this account may control; [own] marks the group the
 * controlled player is already in (shown first, not a choice).
 */
data class SyncTarget(val playerId: String, val memberIds: List<String>, val own: Boolean)

/** Why groups can't be changed now. */
enum class SyncBlock { OFFLINE, BUSY }

/** The controlled player joining another player's group, from its own point of view. */
object JoinGroup {
    /** The others in the controlled player's group, in LMS's order (empty while it plays alone). */
    fun partners(state: PlayerUiState): List<String> {
        val me = state.selectedId ?: return emptyList()
        val known = state.players.mapTo(HashSet()) { it.id }
        return state.syncGroups.firstOrNull { me in it.playerIds }?.playerIds.orEmpty()
            .filter { it != me && it in known }
            .distinct()
    }

    /**
     * What the controlled player may join: every other listed player, a group as one entry,
     * and its own group (if any) first. Ids LMS lists that aren't players this account may
     * control are left out.
     */
    fun targets(state: PlayerUiState): List<SyncTarget> {
        val me = state.selectedId ?: return emptyList()
        val known = state.players.mapTo(HashSet()) { it.id }
        val seen = HashSet<String>()
        val out = mutableListOf<SyncTarget>()
        for (p in state.players) {
            if (p.id in seen) continue
            val members = state.syncGroups.firstOrNull { p.id in it.playerIds }?.playerIds
                ?.filter { it in known }?.distinct()?.takeIf { it.size > 1 }
                ?: listOf(p.id)
            seen += members
            // The controlled player on its own: nothing to join.
            if (members.none { it != me }) continue
            out += SyncTarget(members.first(), members, own = me in members)
        }
        return out.sortedByDescending { it.own }
    }

    /** Why groups can't be changed now (null: they can). */
    fun block(state: PlayerUiState): SyncBlock? = when {
        state.reachability == Reachability.BUSY -> SyncBlock.BUSY
        state.reachability == Reachability.UNREACHABLE || !state.gate.other -> SyncBlock.OFFLINE
        else -> null
    }
}
