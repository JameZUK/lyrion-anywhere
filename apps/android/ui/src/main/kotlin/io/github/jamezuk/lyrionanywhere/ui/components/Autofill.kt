package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.compose.ui.autofill.ContentType

/**
 * The two-step code field, for autofill: a code from an authenticator app (AndroidX's
 * `HintConstants.AUTOFILL_HINT_2FA_APP_OTP`). Never an SMS code: the server sends no texts,
 * and autofill would offer whatever six digits last arrived by message.
 */
val TwoStepCodeContentType: ContentType = ContentType("2faAppOTPCode")
