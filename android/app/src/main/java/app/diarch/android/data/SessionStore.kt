package app.diarch.android.data

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

private val Context.dataStore: DataStore<Preferences> by preferencesDataStore(name = "diarch_session")

class SessionStore(private val context: Context) {
    private val baseUrlKey = stringPreferencesKey("base_url")
    private val tokenKey = stringPreferencesKey("token")
    private val usernameKey = stringPreferencesKey("username")

    val session: Flow<Session> = context.dataStore.data.map { prefs ->
        Session(
            baseUrl = prefs[baseUrlKey].orEmpty(),
            token = prefs[tokenKey].orEmpty(),
            username = prefs[usernameKey].orEmpty(),
        )
    }

    suspend fun saveLogin(baseUrl: String, token: String, username: String) {
        context.dataStore.edit { prefs ->
            prefs[baseUrlKey] = normalizeBaseUrl(baseUrl)
            prefs[tokenKey] = token
            prefs[usernameKey] = username
        }
    }

    suspend fun saveBaseUrl(baseUrl: String) {
        context.dataStore.edit { prefs ->
            prefs[baseUrlKey] = normalizeBaseUrl(baseUrl)
        }
    }

    suspend fun clearToken() {
        context.dataStore.edit { prefs ->
            prefs.remove(tokenKey)
        }
    }

    companion object {
        fun normalizeBaseUrl(raw: String): String {
            var url = raw.trim().trimEnd('/')
            if (url.isNotEmpty() && !url.startsWith("http://") && !url.startsWith("https://")) {
                url = "http://$url"
            }
            return url
        }
    }
}

data class Session(
    val baseUrl: String = "",
    val token: String = "",
    val username: String = "",
) {
    val isLoggedIn: Boolean get() = token.isNotBlank() && baseUrl.isNotBlank()
}
