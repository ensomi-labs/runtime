plugins {
    id("com.android.library") version "9.4.1"
    // Select the compiler used by AGP's built-in Kotlin support.
    id("org.jetbrains.kotlin.android") version "2.4.21" apply false
}

layout.buildDirectory.set(file("../build/android/gradle"))

android {
    namespace = "io.ensomi.runtime"
    compileSdk = 37
    buildToolsVersion = "37.0.0"
    ndkVersion = "30.0.16248370"

    defaultConfig {
        minSdk = 33
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += "arm64-v8a" }
        consumerProguardFiles("consumer-rules.pro")
    }
    testOptions { targetSdk = 36 }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

androidComponents.onVariants { variant ->
    variant.sources.kotlin!!.addStaticSourceDirectory("../build/android/kotlin")
    variant.sources.jniLibs!!.addStaticSourceDirectory("../build/android/jni")
    variant.sources.assets!!.addStaticSourceDirectory("../build/android/assets")
}

kotlin {
    compilerOptions { jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17 }
}

dependencies {
    implementation("net.java.dev.jna:jna:5.19.1@aar")
    compileOnly("androidx.annotation:annotation:1.9.1")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
