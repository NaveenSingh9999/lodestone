package com.lodestone.app.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.material3.Typography
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

// Moss & limestone: the app's own identity, used wherever the device
// can't supply a dynamic palette. Green = the live server world,
// straw amber = activity states, stone neutrals everywhere else.
private val MossPrimary = Color(0xFFA6C58A)
private val MossOnPrimary = Color(0xFF1F2A12)
private val MossContainer = Color(0xFF33471E)
private val OnMossContainer = Color(0xFFC2E3A6)

private val LimestoneSecondary = Color(0xFFBBC4A8)
private val OnLimestone = Color(0xFF262B1E)
private val LimestoneContainer = Color(0xFF3C4233)
private val OnLimestoneContainer = Color(0xFFD6E0C2)

private val StrawTertiary = Color(0xFFE4C284)
private val OnStraw = Color(0xFF3F2D05)
private val StrawContainer = Color(0xFF5A431A)
private val OnStrawContainer = Color(0xFFFBDDA3)

private val StoneNightSurface = Color(0xFF11140C)
private val StoneNightOnSurface = Color(0xFFE4E3DA)
private val StoneNightVariant = Color(0xFF43493A)
private val OnStoneNightVariant = Color(0xFFC3C8B4)
private val StoneNightOutline = Color(0xFF8D9280)

private val StoneDaySurface = Color(0xFFF9FAEF)
private val StoneDayOnSurface = Color(0xFF1A1C16)
private val StoneDayVariant = Color(0xFFE0E4D3)
private val OnStoneDayVariant = Color(0xFF44483D)
private val StoneDayOutline = Color(0xFF747F69)
private val DayPrimary = Color(0xFF4E6B27)
private val OnDayPrimary = Color(0xFFFFFFFF)
private val DayPrimaryContainer = Color(0xFFD0EEAA)
private val OnDayPrimaryContainer = Color(0xFF123300)

private val ErrorRed = Color(0xFFB3554D)

val LodestoneDark = darkColorScheme(
    primary = MossPrimary,
    onPrimary = MossOnPrimary,
    primaryContainer = MossContainer,
    onPrimaryContainer = OnMossContainer,
    secondary = LimestoneSecondary,
    onSecondary = OnLimestone,
    secondaryContainer = LimestoneContainer,
    onSecondaryContainer = OnLimestoneContainer,
    tertiary = StrawTertiary,
    onTertiary = OnStraw,
    tertiaryContainer = StrawContainer,
    onTertiaryContainer = OnStrawContainer,
    background = StoneNightSurface,
    onBackground = StoneNightOnSurface,
    surface = StoneNightSurface,
    onSurface = StoneNightOnSurface,
    surfaceVariant = StoneNightVariant,
    onSurfaceVariant = OnStoneNightVariant,
    outline = StoneNightOutline,
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
    errorContainer = Color(0xFF93000A),
    onErrorContainer = Color(0xFFFFDAD6),
)

val LodestoneLight = lightColorScheme(
    primary = DayPrimary,
    onPrimary = OnDayPrimary,
    primaryContainer = DayPrimaryContainer,
    onPrimaryContainer = OnDayPrimaryContainer,
    secondary = Color(0xFF56614B),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFDAE6C8),
    onSecondaryContainer = Color(0xFF141E0C),
    tertiary = Color(0xFF6F5A29),
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFFF7DE91),
    onTertiaryContainer = Color(0xFF251A00),
    background = StoneDaySurface,
    onBackground = StoneDayOnSurface,
    surface = StoneDaySurface,
    onSurface = StoneDayOnSurface,
    surfaceVariant = StoneDayVariant,
    onSurfaceVariant = OnStoneDayVariant,
    outline = StoneDayOutline,
    error = ErrorRed,
    onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFFFDAD6),
    onErrorContainer = Color(0xFF410002),
)

private val LodestoneTypography = Typography(
    headlineSmall = TextStyle(fontWeight = FontWeight.SemiBold, lineHeight = 30.sp),
    titleLarge = TextStyle(fontWeight = FontWeight.SemiBold, fontSize = 21.sp),
    titleMedium = TextStyle(fontWeight = FontWeight.SemiBold, fontSize = 16.sp),
    labelLarge = TextStyle(fontWeight = FontWeight.SemiBold, fontSize = 14.sp),
)

/** State colors for Console chips / dots. */
object StateColors {
    val Running = Color(0xFF7FBF6A)
    val Starting = Color(0xFFE4C284)
    val Stopped = Color(0xFF8D9280)
    val Crashed = Color(0xFFE57368)
}

fun stateColor(state: String): Color = when (state) {
    "Running" -> StateColors.Running
    "Starting" -> StateColors.Starting
    "Crashed" -> StateColors.Crashed
    else -> StateColors.Stopped
}

@Composable
fun LodestoneTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = true,
    content: @Composable () -> Unit
) {
    val context = LocalContext.current
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        darkTheme -> LodestoneDark
        else -> LodestoneLight
    }
    MaterialTheme(
        colorScheme = colorScheme,
        typography = LodestoneTypography,
        content = content
    )
}
