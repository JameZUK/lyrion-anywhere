package io.github.jamezuk.lyrionanywhere.playback.auto

import android.content.Context
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.os.Process
import java.security.MessageDigest

/** What a controller connecting to the media session may do. */
enum class CallerAccess {
    /** Browse the library and play from it, and control playback: Android Auto, Android
     *  Automotive's media centre, the system's own controllers, and this app. */
    BROWSE,

    /** Control playback only (play, pause, skip), never the library: other controllers the
     *  platform trusts for media, such as a watch app the user gave notification access. */
    CONTROL,

    /** Nothing. */
    REJECT,
}

/** What Android says about a caller (gathered by [CallerFactsReader]); plain values. */
data class CallerFacts(
    val packageName: String,
    val uid: Int,
    /** The package really belongs to that uid. */
    val packageMatchesUid: Boolean,
    /** This app itself. */
    val self: Boolean,
    /** Android's system uid (or root). */
    val systemUid: Boolean,
    /** Signed with the platform's key (system UI, Bluetooth, Automotive's media centre). */
    val platformSigned: Boolean,
    /** Holds MEDIA_CONTENT_CONTROL, a privileged permission only system apps can have. */
    val mediaContentControl: Boolean,
    /** SHA-256 digests of its signing certificates (lower case, colon-separated). */
    val certificates: Set<String>,
    /** Media3 counts it as trusted for media (MEDIA_CONTENT_CONTROL, or an enabled
     *  notification listener). */
    val media3Trusted: Boolean,
)

/** A known car or assistant app: its package and the certificates it is signed with. */
data class KnownCaller(val packageName: String, val name: String, val certificates: Set<String>)

/**
 * Who may connect to the exported media library service (Media3's guidance for
 * `onConnect`: check the caller, never trust a package name alone).
 *
 * - **Browse** (library and playback): this app; Android's system uid; anything signed with
 *   the platform key or holding MEDIA_CONTENT_CONTROL (privileged system apps, such as
 *   Android Automotive's media centre, system UI and Bluetooth); and the known car and
 *   assistant apps of [KNOWN], **only** when signed with one of their release certificates.
 * - **Control** only: other controllers Media3 trusts (an enabled notification listener).
 * - **Reject**: everyone else, and any caller whose package doesn't belong to its uid.
 *
 * Pure: unit-tested on the JVM.
 */
object CallerPolicy {
    /**
     * Release certificates (SHA-256 of the signing certificate) of the apps that browse
     * media from cars, as published in Android's media sample (UAMP,
     * `allowed_media_browser_callers.xml`). Debug keys are deliberately left out.
     */
    val KNOWN: List<KnownCaller> = listOf(
        KnownCaller(
            "com.google.android.projection.gearhead",
            "Android Auto",
            setOf(
                "fd:b0:0c:43:db:de:8b:51:cb:31:2a:a8:1d:3b:5f:a1:77:13:ad:b9:4b:28:f5:98:d7:7f:8e:b8:9d:ac:ee:df",
                // The rotated release key.
                "1c:a8:dc:c0:be:d3:cb:d8:72:d2:cb:79:12:00:c0:29:2c:a9:97:57:68:a8:2d:67:6b:8b:42:4f:b6:5b:52:95",
            ),
        ),
        KnownCaller(
            "com.google.android.googlequicksearchbox",
            "Google Assistant",
            setOf("f0:fd:6c:5b:41:0f:25:cb:25:c3:b5:33:46:c8:97:2f:ae:30:f8:ee:74:11:df:91:04:80:ad:6b:2d:60:db:83"),
        ),
        KnownCaller(
            "com.google.android.carassistant",
            "Google Assistant on Android Automotive",
            setOf("74:b6:fb:f7:10:e8:d9:0d:44:d3:40:12:58:89:b4:23:06:a6:2c:43:79:d0:e5:a6:62:20:e3:a6:8a:bf:90:e2"),
        ),
    )

    fun decide(f: CallerFacts, known: List<KnownCaller> = KNOWN): CallerAccess {
        if (!f.packageMatchesUid || f.packageName.isEmpty()) return CallerAccess.REJECT
        if (f.self || f.systemUid || f.platformSigned || f.mediaContentControl) return CallerAccess.BROWSE
        val k = known.firstOrNull { it.packageName == f.packageName }
        if (k != null && f.certificates.any { it in k.certificates }) return CallerAccess.BROWSE
        // A known package name with the wrong certificate is an impostor: nothing at all.
        if (k != null) return CallerAccess.REJECT
        if (f.media3Trusted) return CallerAccess.CONTROL
        return CallerAccess.REJECT
    }
}

/** "fd:b0:…": the SHA-256 of [bytes], as [CallerPolicy] lists certificates. */
fun sha256Hex(bytes: ByteArray): String =
    MessageDigest.getInstance("SHA-256").digest(bytes).joinToString(":") {
        (it.toInt() and 0xff).toString(16).padStart(2, '0')
    }

/** Reads [CallerFacts] from Android's package manager. */
class CallerFactsReader(private val context: Context) {
    private val pm: PackageManager = context.packageManager

    fun read(packageName: String, uid: Int, media3Trusted: Boolean): CallerFacts {
        val owned = runCatching { pm.getPackagesForUid(uid)?.contains(packageName) == true }.getOrDefault(false)
        val platform = owned && runCatching {
            pm.checkSignatures(PLATFORM_PACKAGE, packageName) == PackageManager.SIGNATURE_MATCH
        }.getOrDefault(false)
        val mcc = owned && runCatching {
            pm.checkPermission(MEDIA_CONTENT_CONTROL, packageName) == PackageManager.PERMISSION_GRANTED
        }.getOrDefault(false)
        return CallerFacts(
            packageName = packageName,
            uid = uid,
            packageMatchesUid = owned,
            self = uid == Process.myUid() && packageName == context.packageName,
            systemUid = uid == Process.SYSTEM_UID || uid == ROOT_UID,
            platformSigned = platform,
            mediaContentControl = mcc,
            certificates = if (owned) certificates(packageName) else emptySet(),
            media3Trusted = media3Trusted,
        )
    }

    /** Every signing certificate of the package (its lineage too, for rotated keys). */
    @Suppress("DEPRECATION")
    private fun certificates(packageName: String): Set<String> {
        val out = HashSet<String>()
        runCatching {
            val info: PackageInfo = pm.getPackageInfo(packageName, PackageManager.GET_SIGNING_CERTIFICATES)
            val s = info.signingInfo
            if (s != null) {
                val certs = if (s.hasMultipleSigners()) s.apkContentsSigners else s.signingCertificateHistory
                certs?.forEach { out += sha256Hex(it.toByteArray()) }
            }
        }
        runCatching {
            pm.getPackageInfo(packageName, PackageManager.GET_SIGNATURES).signatures?.forEach {
                out += sha256Hex(it.toByteArray())
            }
        }
        return out
    }

    private companion object {
        const val PLATFORM_PACKAGE = "android"
        const val MEDIA_CONTENT_CONTROL = "android.permission.MEDIA_CONTENT_CONTROL"
        const val ROOT_UID = 0
    }
}
