package dev.uet.embedfix

import android.app.Activity
import android.os.Bundle
import android.widget.Button
import android.widget.EditText

class MainActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        val input = findViewById<EditText>(R.id.fix_base_input)
        input.setText(Settings.fixBase(this))

        findViewById<Button>(R.id.save_button).setOnClickListener {
            val saved = Settings.saveFixBase(this, input.text.toString())
            if (saved == null) {
                Clip.toast(this, R.string.toast_invalid_url)
            } else {
                input.setText(saved)
                Clip.toast(this, R.string.toast_saved)
            }
        }
    }
}
