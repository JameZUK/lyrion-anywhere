"""Invented text for the demo LMS: artist biographies, album reviews and lyrics.

Every artist, album and word here is made up for the demo, so screenshots taken against it
show nothing real. The Music & Artist Information plugin would normally supply this kind of
text (from online sources, under the server owner's settings).
"""

BIOS = {
    "The Paper Lanterns": (
        "The Paper Lanterns are a four-piece from a small harbour town, formed when two "
        "ferry deckhands started swapping songs on the night crossing. Their sound pairs "
        "close harmonies with brushed drums, pedal steel and the occasional foghorn sample.\n\n"
        "After two self-released EPs recorded in a disused chandlery, the band toured "
        "coastal folk clubs for three winters before making Night Ferry, a song cycle about "
        "leaving home by sea. It became a word-of-mouth favourite on late-night radio and "
        "earned them a reputation as one of the warmest live acts around.\n\n"
        "They still record most of their music in one room, live, with the windows open."
    ),
    "Marisol Vega": (
        "Marisol Vega is a singer, songwriter and keen amateur botanist who writes most of "
        "her songs in a Victorian glasshouse she restored herself. Her records mix nylon-"
        "string guitar, field recordings of rain on glass and spare, close-miked vocals.\n\n"
        "Glasshouse, her third album, was written over one year, a song a month, each named "
        "after something growing at the time."
    ),
    "Northbound Signal": (
        "Northbound Signal is the electronic project of a former radio engineer who builds "
        "synthesisers from salvaged broadcast equipment. Tracks are assembled from shortwave "
        "static, number stations and slow, melodic arpeggios.\n\n"
        "Static Bloom was recorded on a hilltop over a single summer, with the transmitter "
        "hum left in."
    ),
    "Ada & the Undertow": (
        "Ada & the Undertow make sea shanties for people who have never been to sea. Led by "
        "a former choir director, the group sings in rough four-part harmony over accordion, "
        "upright bass and a stomp box made from a fish crate.\n\n"
        "Saltwater Hymns collects the songs they have played at harbour festivals for ten "
        "years, finally recorded in a church hall with its famously generous echo."
    ),
    "Kestrel Lane": (
        "Kestrel Lane writes small songs about big cities at night: night buses, all-night "
        "cafés and the first train home. A drum machine, a borrowed Rhodes piano and a voice "
        "just above a whisper carry most of them.\n\n"
        "Small Hours was recorded between midnight and dawn over twelve nights."
    ),
    "Orchard Quartet": (
        "The Orchard Quartet is a string quartet founded by four friends from the same "
        "conservatoire year, who play contemporary chamber music alongside their own "
        "compositions. They are known for performing in unusual places: barns, lighthouses "
        "and, once, a frozen orchard.\n\n"
        "Winter Études is their first album of original work, four études separated by short "
        "pieces inspired by frost and snow."
    ),
    "Lumen Drift": (
        "Lumen Drift is a duo making slow, glowing electronic music with modular synthesisers "
        "and old projector bulbs used as light-sensitive controllers. Their live shows are "
        "played in near darkness.\n\n"
        "Parallax is a suite about light bending: each track is built from one sound, refracted "
        "through a different chain of effects."
    ),
    "The Copper Hours": (
        "The Copper Hours are a sunny indie-pop band with a love of cassette tape, four-track "
        "recorders and long summer drives. Fuzzy guitars, handclaps and singalong choruses are "
        "their stock in trade.\n\n"
        "Analogue Summer was recorded entirely to tape in a converted garage, hiss and all."
    ),
    "Haruka Mori": (
        "Haruka Mori is a composer and pianist whose ambient pieces are built from recordings "
        "of rock pools, tides and the creak of moored boats, played back slowly beneath "
        "softly pedalled piano.\n\n"
        "Tidal Lines follows a single tide from low water to flood over six pieces, each "
        "recorded at the same beach at a different hour."
    ),
    "Brass Atlas": (
        "Brass Atlas is a loose collective of brass players and sound recordists who travel "
        "with portable recorders and a tuba. Their albums alternate composed pieces with "
        "untouched recordings of the places they played in.\n\n"
        "Field Recordings gathers a year of walks through meadows and hedgerows, with guest "
        "players from every village they passed through."
    ),
    "Selene Park": (
        "Selene Park is a jazz singer and pianist who grew up listening to late-night radio "
        "request shows. Her songs are written as if for a DJ's last hour on air: warm, "
        "unhurried and a little lonely.\n\n"
        "Blue Hour Radio is a concept album framed as one night's broadcast, from the opening "
        "jingle to the sign-off."
    ),
    "Tin Roof Choir": (
        "The Tin Roof Choir started as a group of friends singing on a porch during summer "
        "storms. Today it is a twelve-voice folk choir with banjo, fiddle and harmonium.\n\n"
        "Hymns for the Road brings together traveller's songs from many contributors, "
        "recorded on a tour of village halls."
    ),
}

REVIEWS = {
    "Night Ferry": (
        "Night Ferry is a record that rewards the patience it asks for. Over eight songs the "
        "Paper Lanterns chart a single crossing, from the harbour lights of the opener to the "
        "quiet relief of Home Port, and the sequencing is as careful as the playing.\n\n"
        "The title track is the centrepiece: a slow build of brushed snare, pedal steel and "
        "four-part harmony that never quite resolves, like a ship that won't dock. Undertow "
        "and Gulls at Dawn are just as strong, and the production, recorded live in one room, "
        "leaves space for every instrument to breathe.\n\n"
        "It is the kind of album you put on at night and find yourself still listening to "
        "when it ends. Highly recommended."
    ),
    "Glasshouse": (
        "Glasshouse is gentle, precise and quietly ambitious. Marisol Vega's songs are built "
        "from very little: a guitar, rain on glass and a voice recorded so close you can hear "
        "her breathe. Morning Light and Petrichor are the highlights, but the whole record "
        "holds together as a year in one small, green room."
    ),
    "Static Bloom": (
        "Static Bloom turns radio interference into something unexpectedly beautiful. The "
        "arpeggios of Transmission and Low Orbit glow through the noise, and the closing "
        "Northbound is as moving as electronic music gets."
    ),
    "Small Hours": (
        "Small Hours sounds exactly like its title: twelve nights of city songs, hushed and "
        "glowing. Night Bus is the standout, a three-minute short story set to a drum machine."
    ),
    "Parallax": (
        "Parallax is a slow-motion light show for the ears. Each track takes one sound and "
        "bends it through a different chain of effects, and the result is meditative without "
        "ever becoming background music."
    ),
    "Tidal Lines": (
        "Tidal Lines is ambient music with a sense of place. Haruka Mori's piano drifts over "
        "recordings of rock pools and moorings, and by the time Flood arrives the tide feels "
        "as if it has come in around you."
    ),
}

# Synced lyrics (LRC) for some of the Night Ferry tracks, written for the demo.
LYRICS = {
    "Night Ferry": """[00:12.00]The harbour lights are folding up like paper
[00:19.50]The gulls have gone to sleep along the pier
[00:27.00]I've packed the things I'll need and left the rest behind me
[00:34.50]There's nothing left to keep me waiting here
[00:42.00]
[00:44.00]So take the night ferry, take the long way round
[00:51.50]Count the lanterns as they leave the sound
[00:59.00]Every mile of water is a mile I've found
[01:06.50]On the night ferry, homeward bound
[01:14.00]
[01:16.00]The engine hums a hymn below the decking
[01:23.50]The stars are swinging on the mast above
[01:31.00]I write your name in salt along the railing
[01:38.50]And watch the spray come up and wash it off
[01:46.00]
[01:48.00]So take the night ferry, take the long way round
[01:55.50]Count the lanterns as they leave the sound
[02:03.00]Every mile of water is a mile I've found
[02:10.50]On the night ferry, homeward bound
[02:18.00]
[02:20.00]And when the morning finds us on the far side
[02:27.50]I'll know the way by heart, I'll know the tide
[02:35.00]The paper moons will fade into the daylight
[02:42.50]And you'll be waiting where the harbour lights
""",
    "Harbour Lights": """[00:08.00]Little lights along the harbour wall
[00:15.00]One for every boat that's out tonight
[00:22.00]If you're listening, if you hear me call
[00:29.00]Follow back the harbour lights
[00:36.00]
[00:38.00]Rope and tar and paint and rain
[00:45.00]Every evening much the same
[00:52.00]Count them out and count them in again
[00:59.00]Follow back the harbour lights
""",
    "Paper Moons": """Paper moons hung on a string
Swaying when the ferry sways
Every one a borrowed thing
Every one a wish for better days

Fold another, fold it small
Hang it where the window was
Paper moons won't light at all
But I keep making them because
""",
}
