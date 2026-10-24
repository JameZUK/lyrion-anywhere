package io.github.jamezuk.lyrionanywhere.data

/**
 * Plain `http://` servers (docs/plans/trusted-networks.md).
 *
 * The Rust layer (`parse_server_url`, using `core::privnet`) already refuses any address but
 * `https://`, `http://` to loopback, and `http://` to a *literal* private address (a server
 * on the user's own VPN). On top of that this build decides:
 * - loopback over `http://` only in a debug build (a gateway on this device, for development);
 * - a private address over `http://` only once the user has confirmed, after a warning;
 * - anything else over `http://` never.
 *
 * The private ranges are checked again here, on the normalised address the Rust layer
 * returned, so a mistake there (or an address that didn't come through it) can't send a
 * password in clear to a name or a public address. The same vectors as the Rust, Perl and
 * JavaScript checks (`web/app/tests/fixtures/private-networks.json`) are run against it.
 */
enum class ServerTransport {
    /** `https://`. */
    ENCRYPTED,

    /** `http://` to this device (localhost, 127.0.0.0/8, ::1). */
    LOOPBACK,

    /** `http://` to a literal private address (10.8.0.1, fd00::1, …). */
    PRIVATE_NETWORK,

    /** Anything else: `http://` to a name or a public address, or not an address at all. */
    REFUSED,
}

/** Plain HTTP this build never takes (a name, a public address, or loopback in release). */
val PLAIN_HTTP_REFUSED = AppError.Reason.PLAIN_HTTP.error()

/** Plain HTTP to a private address the user hasn't yet agreed to. */
val UNENCRYPTED_UNCONFIRMED = AppError.Reason.CONFIRM_UNENCRYPTED.error()

/** Whether [error] means "ask the user about an unencrypted connection, then try again". */
fun AppError.needsUnencryptedConfirmation(): Boolean =
    this is AppError.InvalidInput && reason == AppError.Reason.CONFIRM_UNENCRYPTED

/** How [url] (a server address, normally as the Rust layer normalised it) is reached. */
fun serverTransport(url: String): ServerTransport {
    if (url.length > 2048) return ServerTransport.REFUSED
    val scheme = url.substringBefore("://", "").lowercase()
    if (scheme == "https" && url.length > "https://".length) return ServerTransport.ENCRYPTED
    if (scheme != "http") return ServerTransport.REFUSED
    val authority = url.substring("http://".length).substringBefore('/')
    if (authority.isEmpty() || authority.any { it == '@' || it == '?' || it == '#' || it == '\\' || it.isWhitespace() || it.isISOControl() }) {
        return ServerTransport.REFUSED
    }
    val host: String
    val rest: String
    if (authority.startsWith('[')) {
        val end = authority.indexOf(']')
        if (end < 0) return ServerTransport.REFUSED
        host = authority.substring(1, end)
        rest = authority.substring(end + 1)
    } else {
        host = authority.substringBefore(':')
        rest = authority.substring(host.length)
    }
    if (rest.isNotEmpty() && !(rest.startsWith(':') && rest.length in 2..6 && rest.drop(1).all { it in '0'..'9' })) {
        return ServerTransport.REFUSED
    }
    if (host.equals("localhost", ignoreCase = true)) return ServerTransport.LOOPBACK
    val bytes = (if (authority.startsWith('[')) parseIpv6(host) else parseIpv4(host))
        ?: return ServerTransport.REFUSED
    return when {
        isLoopback(bytes) -> ServerTransport.LOOPBACK
        isPrivateAddress(bytes) -> ServerTransport.PRIVATE_NETWORK
        else -> ServerTransport.REFUSED
    }
}

/**
 * What stops this build using the server at [url] now, or null if nothing does.
 * [allowPlainLoopback]: a debug build. [confirmedUnencrypted]: the user has agreed to an
 * unencrypted connection to this (private) server.
 */
fun plainHttpRefusal(url: String, allowPlainLoopback: Boolean, confirmedUnencrypted: Boolean): AppError.InvalidInput? =
    when (serverTransport(url)) {
        ServerTransport.ENCRYPTED -> null
        ServerTransport.LOOPBACK -> if (allowPlainLoopback) null else PLAIN_HTTP_REFUSED
        ServerTransport.PRIVATE_NETWORK -> if (confirmedUnencrypted) null else UNENCRYPTED_UNCONFIRMED
        ServerTransport.REFUSED -> PLAIN_HTTP_REFUSED
    }

/** A dotted-quad IPv4 address (as normalised: four decimal parts, 0 to 255), or null. */
internal fun parseIpv4(s: String): ByteArray? {
    val parts = s.split('.')
    if (parts.size != 4) return null
    val out = ByteArray(4)
    for ((i, p) in parts.withIndex()) {
        // No leading zeros: URL parsers read "010" as octal (8), so it is not the 10 it looks.
        if (p.isEmpty() || p.length > 3 || !p.all { it in '0'..'9' } || (p.length > 1 && p[0] == '0')) return null
        val v = p.toInt()
        if (v > 255) return null
        out[i] = v.toByte()
    }
    return out
}

/**
 * An IPv6 address in hexadecimal groups, with at most one `::`, or null. A dotted IPv4 tail
 * (`::ffff:10.0.0.1`) and zone ids are not accepted (refusing is the safe answer; the Rust
 * layer writes mapped addresses in hexadecimal, and they are never private anyway).
 */
internal fun parseIpv6(s: String): ByteArray? {
    if (s.isEmpty() || s.length > 39 || !s.all { it == ':' || it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }) return null
    val halves = s.split("::")
    if (halves.size > 2) return null
    fun groups(part: String): List<Int>? {
        if (part.isEmpty()) return emptyList()
        return part.split(':').map { g ->
            if (g.isEmpty() || g.length > 4) return null
            g.toInt(16)
        }
    }
    val head = groups(halves[0]) ?: return null
    val tail = if (halves.size == 2) groups(halves[1]) ?: return null else emptyList()
    val all = if (halves.size == 2) {
        if (head.size + tail.size > 7) return null
        head + List(8 - head.size - tail.size) { 0 } + tail
    } else {
        if (head.size != 8) return null
        head
    }
    val out = ByteArray(16)
    for ((i, g) in all.withIndex()) {
        out[2 * i] = (g shr 8).toByte()
        out[2 * i + 1] = g.toByte()
    }
    return out
}

private fun ByteArray.u(i: Int) = this[i].toInt() and 0xff

private fun isLoopback(b: ByteArray): Boolean = when (b.size) {
    4 -> b.u(0) == 127
    16 -> (0 until 15).all { b[it] == 0.toByte() } && b[15] == 1.toByte()
    else -> false
}

/**
 * Inside 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, 100.64.0.0/10,
 * fc00::/7 or fe80::/10 (`core::privnet::PRIVATE_RANGES`).
 */
internal fun isPrivateAddress(b: ByteArray): Boolean = when (b.size) {
    4 -> b.u(0) == 10 ||
        (b.u(0) == 172 && (b.u(1) and 0xf0) == 16) ||
        (b.u(0) == 192 && b.u(1) == 168) ||
        (b.u(0) == 169 && b.u(1) == 254) ||
        (b.u(0) == 100 && (b.u(1) and 0xc0) == 64)
    16 -> ((b.u(0) and 0xfe) == 0xfc) || (b.u(0) == 0xfe && (b.u(1) and 0xc0) == 0x80)
    else -> false
}
