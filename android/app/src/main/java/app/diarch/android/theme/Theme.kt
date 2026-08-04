package app.diarch.android.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.sp
import androidx.compose.material3.Typography

private val Ink = Color(0xFFF3E6D4)
private val Page = Color(0xFF0C1210)
private val Accent = Color(0xFFE8B87A)
private val Muted = Color(0xFF8A9A8E)
private val Surface = Color(0xFF141C18)
private val SurfaceHigh = Color(0xFF1C2620)

private val DarkColors = darkColorScheme(
    primary = Accent,
    onPrimary = Page,
    secondary = Muted,
    onSecondary = Ink,
    background = Page,
    onBackground = Ink,
    surface = Surface,
    onSurface = Ink,
    surfaceVariant = SurfaceHigh,
    onSurfaceVariant = Muted,
    error = Color(0xFFE07070),
    onError = Page,
)

private val LightColors = lightColorScheme(
    primary = Color(0xFF7A4E28),
    onPrimary = Color(0xFFFAFAFA),
    secondary = Color(0xFF5B4636),
    onSecondary = Color(0xFFF7F1E5),
    background = Color(0xFFF7F1E5),
    onBackground = Color(0xFF2A241C),
    surface = Color(0xFFFFFBF4),
    onSurface = Color(0xFF2A241C),
    surfaceVariant = Color(0xFFEDE4D4),
    onSurfaceVariant = Color(0xFF5B4636),
)

private val DiarchTypography = Typography(
    displayLarge = TextStyle(
        fontFamily = FontFamily.Serif,
        fontWeight = FontWeight.Bold,
        fontSize = 36.sp,
        lineHeight = 42.sp,
    ),
    headlineMedium = TextStyle(
        fontFamily = FontFamily.Serif,
        fontWeight = FontWeight.SemiBold,
        fontSize = 24.sp,
        lineHeight = 30.sp,
    ),
    titleLarge = TextStyle(
        fontFamily = FontFamily.Serif,
        fontWeight = FontWeight.SemiBold,
        fontSize = 20.sp,
    ),
    bodyLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontSize = 16.sp,
        lineHeight = 24.sp,
    ),
    bodyMedium = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontSize = 14.sp,
        lineHeight = 20.sp,
    ),
    labelLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Medium,
        fontSize = 14.sp,
    ),
)

@Composable
fun DiarchTheme(
    darkTheme: Boolean = true,
    content: @Composable () -> Unit,
) {
    MaterialTheme(
        colorScheme = if (darkTheme || isSystemInDarkTheme()) DarkColors else LightColors,
        typography = DiarchTypography,
        content = content,
    )
}
