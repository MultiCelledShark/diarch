package app.diarch.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import app.diarch.android.data.Session
import app.diarch.android.data.WorkDetailResponse
import app.diarch.android.theme.DiarchTheme
import app.diarch.android.ui.detail.ImportScreen
import app.diarch.android.ui.detail.WorkDetailScreen
import app.diarch.android.ui.login.LoginScreen
import app.diarch.android.ui.reader.ReaderScreen
import app.diarch.android.ui.shelves.ShelvesScreen
import app.diarch.android.ui.wishlist.WishlistAddScreen

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            DiarchTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    DiarchNav()
                }
            }
        }
    }
}

@Composable
private fun DiarchNav() {
    val app = DiarchApp.instance
    val session by app.sessionStore.session.collectAsStateWithLifecycle(initialValue = Session())
    var sessionReady by remember { mutableStateOf(false) }
    var readerDetail by remember { mutableStateOf<WorkDetailResponse?>(null) }

    LaunchedEffect(session.baseUrl, session.token) {
        if (session.baseUrl.isNotBlank()) {
            app.apiClient.updateSession(session.baseUrl, session.token)
        }
        sessionReady = true
    }

    if (!sessionReady) return

    val nav = rememberNavController()
    val start = if (session.isLoggedIn) "shelves" else "login"

    NavHost(navController = nav, startDestination = start) {
        composable("login") {
            LoginScreen(
                initialBaseUrl = session.baseUrl,
                onLoggedIn = {
                    nav.navigate("shelves") {
                        popUpTo("login") { inclusive = true }
                    }
                },
            )
        }
        composable("shelves") {
            ShelvesScreen(
                onOpenWork = { id -> nav.navigate("work/$id") },
                onImport = { nav.navigate("import") },
                onWishlistAdd = { nav.navigate("wishlist-add") },
                onLogout = {
                    nav.navigate("login") {
                        popUpTo(0) { inclusive = true }
                    }
                },
            )
        }
        composable("wishlist-add") {
            WishlistAddScreen(
                onBack = { nav.popBackStack() },
                onAdded = { id ->
                    nav.navigate("work/$id") {
                        popUpTo("shelves")
                    }
                },
            )
        }
        composable(
            route = "work/{id}",
            arguments = listOf(navArgument("id") { type = NavType.StringType }),
        ) { entry ->
            val id = entry.arguments?.getString("id") ?: return@composable
            WorkDetailScreen(
                workId = id,
                onBack = { nav.popBackStack() },
                onRead = { detail ->
                    readerDetail = detail
                    nav.navigate("reader")
                },
            )
        }
        composable("import") {
            ImportScreen(
                onBack = { nav.popBackStack() },
                onImported = { id ->
                    nav.navigate("work/$id") {
                        popUpTo("shelves")
                    }
                },
            )
        }
        composable("reader") {
            val detail = readerDetail
            if (detail == null) {
                LaunchedEffect(Unit) { nav.popBackStack() }
            } else {
                ReaderScreen(
                    detail = detail,
                    onClose = {
                        readerDetail = null
                        nav.popBackStack()
                    },
                )
            }
        }
    }
}
