package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.ui.library.carouselCardWidth
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CarouselSizeTest {
    @Test
    fun cardsFitAboutTwoAndAHalfAcrossAPhoneAndStayInBounds() {
        val phone = carouselCardWidth(411)
        assertTrue("about 2.4 cards on a phone: $phone", (411 - 32) / (phone + 12) in 2.2f..2.6f)
        assertEquals(112f, carouselCardWidth(280), 0.01f)
        assertEquals(190f, carouselCardWidth(1280), 0.01f)
        assertTrue(carouselCardWidth(411, small = true) < phone)
        assertEquals(96f, carouselCardWidth(200, small = true), 0.01f)
    }
}
