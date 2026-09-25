package app.diarch.android.data

import android.content.Context
import android.content.SharedPreferences
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.withContext

private val Context.dataStore: DataStore<Preferences> by preferencesDataStore(name = "diarch_session")

/**
 * The auth token is sensitive and is kept in [EncryptedSharedPreferences] (Android Keystore
 * backed), separate from the plaintext DataStore used for the (non-sensitive) base URL and
 * username.
 */
class SessionStore(private val context: Context) {
    private val baseUrlKey = stringPreferencesKey("base_url")
    private val usernameKey = stringPreferencesKey("username")
    private val isAdminKey = stringPreferencesKey("is_admin")
    private val tokenKey = "token"

    private val encryptedPrefs: SharedPreferences by lazy { buildEncryptedPrefs(context) }

    private val tokenState = MutableStateFlow(readTokenSync())

    val session: Flow<Session> = combine(context.dataStore.data, tokenState) { prefs, token ->
        Session(
            baseUrl = prefs[baseUrlKey].orEmpty(),
            token = token,
            username = prefs[usernameKey].orEmpty(),
            isAdmin = prefs[isAdminKey] == "1",
        )
    }

    suspend fun saveLogin(baseUrl: String, token: String, username: String, isAdmin: Boolean = false) {
        writeToken(token)
        context.dataStore.edit { prefs ->
            prefs[baseUrlKey] = normalizeBaseUrl(baseUrl)
            prefs[usernameKey] = username
            prefs[isAdminKey] = if (isAdmin) "1" else "0"
        }
    }

    suspend fun saveBaseUrl(baseUrl: String) {
        context.dataStore.edit { prefs ->
            prefs[baseUrlKey] = normalizeBaseUrl(baseUrl)
        }
    }

    suspend fun clearToken() {
        writeToken("")
        context.dataStore.edit { prefs ->
            prefs.remove(isAdminKey)
        }
    }

    /** Refresh admin flag without touching the auth token (e.g. after `/api/auth/me`). */
    suspend fun setIsAdmin(isAdmin: Boolean) {
        context.dataStore.edit { prefs ->
            prefs[isAdminKey] = if (isAdmin) "1" else "0"
        }
    }

    private fun readTokenSync(): String = encryptedPrefs.getString(tokenKey, "").orEmpty()

    private suspend fun writeToken(token: String) {
        withContext(Dispatchers.IO) {
            encryptedPrefs.edit().putString(tokenKey, token).apply()
        }
        tokenState.value = token
    }

    private fun buildEncryptedPrefs(context: Context): SharedPreferences {
        val masterKey = MasterKey.Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()
        return EncryptedSharedPreferences.create(
            context,
            "diarch_secure_prefs",
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
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
    val isAdmin: Boolean = false,
) {
    val isLoggedIn: Boolean get() = token.isNotBlank() && baseUrl.isNotBlank()
}
