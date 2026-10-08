plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Override with: ./gradlew assembleDebug -PfixBaseUrl=https://my.host
val fixBaseUrl: String = (project.findProperty("fixBaseUrl") as String?)
    ?.takeIf { it.isNotBlank() }
    ?: "https://fix.example.com"

android {
    namespace = "dev.uet.embedfix"
    compileSdk = 35
    buildToolsVersion = "35.0.0"

    defaultConfig {
        applicationId = "dev.uet.embedfix"
        minSdk = 24
        targetSdk = 35
        versionCode = System.getenv("UET_VERSION_CODE")?.toIntOrNull() ?: 1
        versionName = System.getenv("UET_VERSION_NAME")?.takeIf { it.isNotBlank() } ?: "1.0"

        val escaped = fixBaseUrl.replace("\\", "\\\\").replace("\"", "\\\"")
        buildConfigField("String", "FIX_BASE_URL", "\"$escaped\"")
    }

    buildFeatures {
        buildConfig = true
    }

    // Release signing: only active when all four env vars are set (CI / local release builds).
    // Debug builds and unsigned release builds are unaffected otherwise.
    val ksPath = System.getenv("ANDROID_KEYSTORE_PATH")?.takeIf { it.isNotBlank() }
    val ksPassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")?.takeIf { it.isNotBlank() }
    val ksAlias = System.getenv("ANDROID_KEY_ALIAS")?.takeIf { it.isNotBlank() }
    val ksKeyPassword = System.getenv("ANDROID_KEY_PASSWORD")?.takeIf { it.isNotBlank() }
    val releaseSigning = ksPath != null && ksPassword != null && ksAlias != null && ksKeyPassword != null

    if (releaseSigning) {
        signingConfigs {
            create("release") {
                storeFile = file(ksPath!!)
                storePassword = ksPassword
                keyAlias = ksAlias
                keyPassword = ksKeyPassword
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            if (releaseSigning) signingConfig = signingConfigs.getByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    sourceSets {
        getByName("main").java.srcDir("src/main/kotlin")
        getByName("test").java.srcDir("src/test/kotlin")
    }
}

dependencies {
    testImplementation("junit:junit:4.13.2")
}
