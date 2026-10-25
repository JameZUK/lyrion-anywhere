package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.QualityDiagram
import io.github.jamezuk.lyrionanywhere.core.QualityFile
import io.github.jamezuk.lyrionanywhere.core.QualityInputs
import io.github.jamezuk.lyrionanywhere.core.QualityNetwork
import io.github.jamezuk.lyrionanywhere.core.QualityOrigin
import io.github.jamezuk.lyrionanywhere.core.QualityPlayer
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.core.describeQuality
import io.github.jamezuk.lyrionanywhere.core.localQualityDiagram
import io.github.jamezuk.lyrionanywhere.core.offlineQualityDiagram

/**
 * The quality badge and signal-path diagram (docs/plans/signal-path-redesign.md), from the
 * shared rules in the Rust core. A seam: the screens' tests give their own diagrams.
 */
interface QualityDescriber {
    /** This phone as the player: its signal path (with the gateway's stream message) and what the app knows. */
    fun local(path: SignalPathInfo?, file: QualityFile?, origin: QualityOrigin?, seekable: Boolean): QualityDiagram

    /** Another player: the file only (the gateway never transcodes for it). */
    fun remote(name: String, file: QualityFile?): QualityDiagram

    /** A download played on this phone: Download → This device. */
    fun offline(path: SignalPathInfo?, format: DownloadFormat, codec: String): QualityDiagram
}

/** The Rust core's rules (shared with the web app and held to the same vectors). */
object RustQualityDescriber : QualityDescriber {
    override fun local(path: SignalPathInfo?, file: QualityFile?, origin: QualityOrigin?, seekable: Boolean): QualityDiagram =
        localQualityDiagram(path, file, origin, seekable)

    override fun remote(name: String, file: QualityFile?): QualityDiagram =
        describeQuality(QualityInputs(QualityPlayer.Remote(name), file, null, null, null, null, null, false))

    override fun offline(path: SignalPathInfo?, format: DownloadFormat, codec: String): QualityDiagram =
        offlineQualityDiagram(path, format, codec)
}

/** The lowest player protocol this app can play from (docs/PROTOCOL.md, *Versions*). */
const val MIN_PLAYER_PROTOCOL = 2u

/**
 * Why the level in force is in force: chosen in the sheet for now ([override]), else the
 * setting for this network.
 */
fun qualityOrigin(override: StreamQuality?, setting: StreamQuality, metered: Boolean): QualityOrigin =
    override?.let { QualityOrigin.Override(it) }
        ?: QualityOrigin.Setting(if (metered) QualityNetwork.MOBILE else QualityNetwork.WIFI, setting)
