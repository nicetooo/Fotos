package app.footos.viewer

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.VideoView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.LinearSnapHelper
import androidx.recyclerview.widget.RecyclerView
import androidx.viewpager2.widget.MarginPageTransformer
import androidx.viewpager2.widget.ViewPager2
import java.text.DateFormat
import java.util.Calendar
import kotlin.math.max

/**
 * Native full-screen photo viewer, shown above the WebView.
 *
 * Paging, pinch / double-tap zoom, drag down to dismiss, a bar with the date,
 * a filmstrip whose centred thumbnail is the photo on display, inline video.
 */
class PhotoViewerActivity : AppCompatActivity() {

    /** One entry of the list handed over by the web side. All paths are local files. */
    data class Item(
        val path: String,
        val thumb: String,
        val mediaType: String,
        val dateTaken: String,
    )

    companion object {
        // The list can be large, so it stays in-process instead of going through
        // an Intent (binder transactions are limited to about 1 MB).
        private var pendingItems: List<Item> = emptyList()
        private var pendingIndex = 0
        private var pendingClosed: ((Int) -> Unit)? = null

        fun open(from: Activity, items: List<Item>, index: Int, onClosed: (Int) -> Unit) {
            if (items.isEmpty()) return
            pendingItems = items
            pendingIndex = index.coerceIn(0, items.size - 1)
            pendingClosed = onClosed
            from.startActivity(Intent(from, PhotoViewerActivity::class.java))
            from.overridePendingTransition(android.R.anim.fade_in, 0)
        }
    }

    private lateinit var items: List<Item>
    private var onClosed: ((Int) -> Unit)? = null

    private lateinit var root: FrameLayout
    private lateinit var pager: ViewPager2
    private lateinit var topBar: LinearLayout
    private lateinit var titleView: TextView
    private lateinit var subtitleView: TextView
    private lateinit var strip: RecyclerView
    private val background = ColorDrawable(Color.BLACK)

    private var chromeVisible = true
    private var stripDriving = false
    private var fullSide = 2048

    private fun dp(value: Float): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value, resources.displayMetrics).toInt()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        items = pendingItems
        onClosed = pendingClosed
        pendingItems = emptyList()
        pendingClosed = null
        if (items.isEmpty()) {
            // Recreated without its list (process death): nothing to show.
            finish()
            return
        }

        WindowCompat.setDecorFitsSystemWindows(window, false)
        window.statusBarColor = Color.TRANSPARENT
        window.navigationBarColor = Color.TRANSPARENT

        // Sharp up to about 2x on this screen without holding full-size bitmaps.
        val metrics = resources.displayMetrics
        fullSide = (max(metrics.widthPixels, metrics.heightPixels) * 1.25f).toInt().coerceIn(1600, 3072)

        root = FrameLayout(this)
        root.background = background
        setContentView(root)

        buildPager()
        buildTopBar()
        buildStrip()

        ViewCompat.setOnApplyWindowInsetsListener(root) { _, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            topBar.setPadding(dp(4f), bars.top + dp(4f), dp(4f), dp(20f))
            (strip.layoutParams as FrameLayout.LayoutParams).bottomMargin = bars.bottom + dp(10f)
            strip.requestLayout()
            insets
        }

        val start = savedInstanceState?.getInt("index") ?: pendingIndex
        pager.setCurrentItem(start, false)
        showPage(start)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        if (::pager.isInitialized) outState.putInt("index", pager.currentItem)
    }

    // ------------------------------------------------------------------ pager

    private fun buildPager() {
        pager = ViewPager2(this)
        pager.offscreenPageLimit = 1
        pager.setPageTransformer(MarginPageTransformer(dp(16f)))
        pager.adapter = PageAdapter()
        pager.registerOnPageChangeCallback(object : ViewPager2.OnPageChangeCallback() {
            private var last = -1

            override fun onPageSelected(position: Int) {
                if (last >= 0 && last != position) pageAt(last)?.leave()
                last = position
                showPage(position)
            }
        })
        root.addView(pager, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
    }

    private fun pageAt(position: Int): PageHolder? {
        val recycler = pager.getChildAt(0) as? RecyclerView ?: return null
        return recycler.findViewHolderForAdapterPosition(position) as? PageHolder
    }

    private fun showPage(position: Int) {
        val (date, time) = formatDate(items[position].dateTaken)
        titleView.text = date
        subtitleView.text = time
        subtitleView.visibility = if (time.isEmpty()) View.GONE else View.VISIBLE
        if (!stripDriving) centerStripOn(position)
    }

    private inner class PageAdapter : RecyclerView.Adapter<PageHolder>() {
        override fun getItemCount() = items.size

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): PageHolder {
            val frame = FrameLayout(parent.context)
            frame.layoutParams = ViewGroup.LayoutParams(MATCH_PARENT, MATCH_PARENT)
            return PageHolder(frame)
        }

        override fun onBindViewHolder(holder: PageHolder, position: Int) = holder.bind(items[position])

        override fun onViewRecycled(holder: PageHolder) = holder.leave()
    }

    private inner class PageHolder(val frame: FrameLayout) : RecyclerView.ViewHolder(frame) {
        private val image = ZoomImageView(frame.context)
        private val playGlyph = TextView(frame.context)
        private val liveBadge = TextView(frame.context)
        private var video: VideoView? = null
        private var item: Item? = null

        init {
            frame.addView(image, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))

            playGlyph.text = "▶"
            playGlyph.setTextColor(Color.WHITE)
            playGlyph.setTextSize(TypedValue.COMPLEX_UNIT_SP, 30f)
            playGlyph.gravity = Gravity.CENTER
            playGlyph.background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(0x80000000.toInt())
            }
            frame.addView(playGlyph, FrameLayout.LayoutParams(dp(72f), dp(72f), Gravity.CENTER))

            liveBadge.text = "LIVE"
            liveBadge.setTextColor(Color.WHITE)
            liveBadge.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11f)
            liveBadge.typeface = Typeface.DEFAULT_BOLD
            liveBadge.setPadding(dp(8f), dp(2f), dp(8f), dp(2f))
            liveBadge.background = GradientDrawable().apply {
                cornerRadius = dp(9f).toFloat()
                setColor(0x73000000)
            }
            frame.addView(
                liveBadge,
                FrameLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT, Gravity.TOP or Gravity.START).apply {
                    leftMargin = dp(16f)
                    topMargin = dp(104f)
                },
            )

            image.onTap = { onPageTapped(this) }
            image.onDismissProgress = { progress ->
                background.alpha = ((1f - progress) * 255).toInt()
                val alpha = if (chromeVisible) 1f - progress * 3f else 0f
                topBar.alpha = alpha.coerceIn(0f, 1f)
                strip.alpha = alpha.coerceIn(0f, 1f)
                liveBadge.alpha = alpha.coerceIn(0f, 1f)
            }
            image.onDismiss = { close() }
        }

        val isVideo: Boolean get() = item?.mediaType == "video"

        fun bind(item: Item) {
            leave()
            this.item = item
            image.zoomEnabled = item.mediaType != "video"
            playGlyph.visibility = if (isVideo) View.VISIBLE else View.GONE
            liveBadge.visibility = if (item.mediaType == "live_photo") View.VISIBLE else View.GONE

            image.setImageDrawable(null)
            ImageLoader.cancel(image)
            // Thumbnail first, then the full image over it (videos keep the thumbnail).
            ImageLoader.load(item.thumb, 512, image) {
                if (this.item === item && !isVideo) ImageLoader.load(item.path, fullSide, image)
            }
        }

        fun togglePlayback() {
            val item = item ?: return
            val current = video
            if (current != null) {
                if (current.isPlaying) {
                    current.pause()
                    playGlyph.visibility = View.VISIBLE
                } else {
                    current.start()
                    playGlyph.visibility = View.GONE
                }
                return
            }
            val view = VideoView(frame.context)
            video = view
            frame.addView(view, 1, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT, Gravity.CENTER))
            playGlyph.visibility = View.GONE
            view.setOnPreparedListener { it.isLooping = false }
            view.setOnCompletionListener {
                playGlyph.visibility = View.VISIBLE
                setChrome(true)
            }
            view.setOnErrorListener { _, _, _ ->
                stopVideo()
                true
            }
            // Taps land on the video surface while it is on top of the image.
            view.setOnClickListener { onPageTapped(this) }
            view.setVideoPath(item.path)
            view.start()
        }

        val isPlaying: Boolean get() = video?.isPlaying == true

        private fun stopVideo() {
            val view = video ?: return
            video = null
            view.stopPlayback()
            frame.removeView(view)
            playGlyph.visibility = if (isVideo) View.VISIBLE else View.GONE
        }

        /** The page is no longer the one on display. */
        fun leave() {
            stopVideo()
            image.reset()
        }
    }

    private fun onPageTapped(page: PageHolder) {
        if (page.isVideo) {
            page.togglePlayback()
            setChrome(!page.isPlaying)
        } else {
            setChrome(!chromeVisible)
        }
    }

    // ----------------------------------------------------------------- chrome

    private fun buildTopBar() {
        topBar = LinearLayout(this)
        topBar.orientation = LinearLayout.HORIZONTAL
        topBar.gravity = Gravity.CENTER_VERTICAL
        topBar.background = GradientDrawable(
            GradientDrawable.Orientation.TOP_BOTTOM,
            intArrayOf(0x99000000.toInt(), 0x00000000),
        )

        val back = TextView(this)
        back.text = "‹"
        back.setTextColor(Color.WHITE)
        back.setTextSize(TypedValue.COMPLEX_UNIT_SP, 34f)
        back.gravity = Gravity.CENTER
        back.contentDescription = getString(android.R.string.cancel)
        back.setOnClickListener { close() }
        topBar.addView(back, LinearLayout.LayoutParams(dp(48f), dp(48f)))

        val titles = LinearLayout(this)
        titles.orientation = LinearLayout.VERTICAL
        titles.gravity = Gravity.CENTER_HORIZONTAL
        titleView = TextView(this).apply {
            setTextColor(Color.WHITE)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
            typeface = Typeface.DEFAULT_BOLD
            gravity = Gravity.CENTER
        }
        subtitleView = TextView(this).apply {
            setTextColor(0xCCFFFFFF.toInt())
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
            gravity = Gravity.CENTER
        }
        titles.addView(titleView, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        titles.addView(subtitleView, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        topBar.addView(titles, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))

        // Balances the back button so the title is centred.
        topBar.addView(View(this), LinearLayout.LayoutParams(dp(48f), dp(48f)))

        root.addView(topBar, FrameLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT, Gravity.TOP))
    }

    private fun setChrome(visible: Boolean) {
        if (chromeVisible == visible) return
        chromeVisible = visible
        val alpha = if (visible) 1f else 0f
        for (view in listOf<View>(topBar, strip)) {
            view.animate().alpha(alpha).setDuration(180).start()
        }
        // An invisible strip must not take touches meant for the photo.
        strip.visibility = View.VISIBLE
        if (!visible) strip.postDelayed({ if (!chromeVisible) strip.visibility = View.INVISIBLE }, 190)
        topBar.visibility = View.VISIBLE
        if (!visible) topBar.postDelayed({ if (!chromeVisible) topBar.visibility = View.INVISIBLE }, 190)
    }

    // -------------------------------------------------------------- filmstrip

    private val thumbW get() = dp(34f)
    private val thumbH get() = dp(48f)
    private val thumbGap get() = dp(2f)

    private fun buildStrip() {
        strip = RecyclerView(this)
        val layout = LinearLayoutManager(this, RecyclerView.HORIZONTAL, false)
        strip.layoutManager = layout
        strip.clipToPadding = false
        strip.overScrollMode = View.OVER_SCROLL_NEVER
        strip.adapter = StripAdapter()
        val snap = LinearSnapHelper()
        snap.attachToRecyclerView(strip)

        // Side padding of half the width puts the first and last thumbnail in the centre.
        strip.addOnLayoutChangeListener { _, left, _, right, _, oldLeft, _, oldRight, _ ->
            if (right - left != oldRight - oldLeft) {
                val pad = (right - left - thumbW - thumbGap) / 2
                strip.setPadding(pad, 0, pad, 0)
                centerStripOn(pager.currentItem)
            }
        }

        strip.addOnScrollListener(object : RecyclerView.OnScrollListener() {
            override fun onScrollStateChanged(recyclerView: RecyclerView, newState: Int) {
                // Only a finger on the strip makes it drive the pager.
                if (newState == RecyclerView.SCROLL_STATE_DRAGGING) stripDriving = true
                if (newState == RecyclerView.SCROLL_STATE_IDLE) stripDriving = false
            }

            override fun onScrolled(recyclerView: RecyclerView, dx: Int, dy: Int) {
                if (!stripDriving) return
                val centre = snap.findSnapView(layout) ?: return
                val position = layout.getPosition(centre)
                if (position != RecyclerView.NO_POSITION && position != pager.currentItem) {
                    pager.setCurrentItem(position, false)
                }
            }
        })

        root.addView(strip, FrameLayout.LayoutParams(MATCH_PARENT, thumbH, Gravity.BOTTOM))
    }

    private fun centerStripOn(position: Int) {
        (strip.layoutManager as LinearLayoutManager).scrollToPositionWithOffset(position, 0)
    }

    private inner class StripAdapter : RecyclerView.Adapter<StripHolder>() {
        override fun getItemCount() = items.size

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): StripHolder {
            val view = ImageView(parent.context)
            view.scaleType = ImageView.ScaleType.CENTER_CROP
            view.setBackgroundColor(0xFF222222.toInt())
            view.layoutParams = RecyclerView.LayoutParams(thumbW, thumbH).apply {
                leftMargin = thumbGap / 2
                rightMargin = thumbGap - thumbGap / 2
            }
            return StripHolder(view)
        }

        override fun onBindViewHolder(holder: StripHolder, position: Int) {
            val view = holder.view
            view.setImageDrawable(null)
            ImageLoader.cancel(view)
            val item = items[position]
            if (item.thumb.isNotEmpty()) ImageLoader.load(item.thumb, 128, view)
            view.setOnClickListener {
                val at = holder.adapterPosition
                if (at != RecyclerView.NO_POSITION) pager.setCurrentItem(at, false)
            }
        }
    }

    private class StripHolder(val view: ImageView) : RecyclerView.ViewHolder(view)

    // ------------------------------------------------------------------ close

    private fun close() {
        finish()
    }

    override fun finish() {
        val callback = onClosed
        onClosed = null
        val index = if (::pager.isInitialized) pager.currentItem else 0
        super.finish()
        overridePendingTransition(0, android.R.anim.fade_out)
        callback?.invoke(index)
    }

    override fun onDestroy() {
        // Closed by the system rather than through finish(): still report it.
        val callback = onClosed
        onClosed = null
        if (callback != null && isFinishing) callback(if (::pager.isInitialized) pager.currentItem else 0)
        super.onDestroy()
    }

    // ------------------------------------------------------------------ dates

    /** "yyyy-MM-dd HH:mm:ss" (or with ':' / 'T' separators) as a local date and a time line. */
    private fun formatDate(raw: String): Pair<String, String> {
        val m = Regex("""^"?(\d{4})[-:](\d{2})[-:](\d{2})(?:[ T](\d{2}):(\d{2}))?""").find(raw.trim())
            ?: return Pair("", "")
        val (y, mo, d, h, mi) = m.destructured
        val calendar = Calendar.getInstance()
        calendar.clear()
        calendar.set(y.toInt(), mo.toInt() - 1, d.toInt(), h.toIntOrNull() ?: 0, mi.toIntOrNull() ?: 0)
        val date = DateFormat.getDateInstance(DateFormat.LONG).format(calendar.time)
        val time = if (h.isEmpty()) "" else DateFormat.getTimeInstance(DateFormat.SHORT).format(calendar.time)
        return Pair(date, time)
    }
}
