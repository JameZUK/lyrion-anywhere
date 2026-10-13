package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.AssistChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.Credit
import io.github.jamezuk.lyrionanywhere.core.RoleCount
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.ui.R

/** A contributor role's name ("Composer"). */
@Composable
fun roleLabel(role: ArtistRole): String = stringResource(
    when (role) {
        ArtistRole.ALBUM_ARTIST -> R.string.role_album_artist
        ArtistRole.ARTIST -> R.string.role_artist
        ArtistRole.TRACK_ARTIST -> R.string.role_track_artist
        ArtistRole.COMPOSER -> R.string.role_composer
        ArtistRole.CONDUCTOR -> R.string.role_conductor
        ArtistRole.BAND -> R.string.role_band
    },
)


/**
 * A track's credits (Wave 2 §3.1): each role LMS reports, then its people. Every name that
 * can be followed is its own link (and its own TalkBack item), to that person's albums in
 * that role; a name LMS gave without an id is shown as plain text.
 */
@Composable
fun CreditsSection(credits: List<Credit>, onCredit: (Credit) -> Unit, modifier: Modifier = Modifier) {
    if (credits.isEmpty()) return
    Column(modifier.fillMaxWidth().testTag("credits")) {
        SectionHeader(stringResource(R.string.section_credits))
        credits.groupBy { it.role }.forEach { (role, people) ->
            val roleName = roleLabel(role)
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    roleName,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.widthIn(min = 104.dp, max = 140.dp).padding(end = 8.dp),
                )
                FlowRow(Modifier.weight(1f), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    people.forEach { c ->
                        if (c.artistId != null) {
                            Text(
                                c.name,
                                style = MaterialTheme.typography.bodyLarge,
                                color = MaterialTheme.colorScheme.primary,
                                // A link: underlined, so it doesn't rely on colour alone.
                                textDecoration = TextDecoration.Underline,
                                modifier = Modifier
                                    .heightIn(min = 48.dp)
                                    .clickable(
                                        role = Role.Button,
                                        onClickLabel = stringResource(R.string.credit_open, roleName),
                                    ) { onCredit(c) }
                                    .padding(vertical = 12.dp)
                                    .testTag("credit-${c.artistId}-${role.name}"),
                            )
                        } else {
                            Text(c.name, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.padding(vertical = 12.dp))
                        }
                    }
                }
            }
        }
    }
}

/**
 * "Appears as" on an artist page: a chip for each role the artist has albums in, with how
 * many; each opens their albums in that role. Nothing when LMS reported no roles.
 */
@Composable
fun AppearsAs(roles: List<RoleCount>, onRole: (ArtistRole) -> Unit, modifier: Modifier = Modifier) {
    if (roles.isEmpty()) return
    val res = LocalResources.current
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("appears-as")) {
        Text(stringResource(R.string.appears_as), style = MaterialTheme.typography.labelLarge)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            roles.forEach { r ->
                val name = roleLabel(r.role)
                val n = r.albums.toInt()
                val spoken = res.getQuantityString(R.plurals.appears_as_chip_description, n, name, n)
                AssistChip(
                    onClick = { onRole(r.role) },
                    label = { Text(stringResource(R.string.appears_as_chip, name, n)) },
                    modifier = Modifier.semantics { contentDescription = spoken }.testTag("role-${r.role.name}"),
                )
            }
        }
    }
}

/** A contributor's albums in one role ("Bach · Composer"). */
class ContributorViewModel(library: LibraryRepository, artistId: Long, role: ArtistRole) :
    LoadViewModel<List<io.github.jamezuk.lyrionanywhere.core.Album>>({ library.artistAlbums(artistId, role) })
