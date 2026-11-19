package app.footos.viewer

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.Matrix
import android.graphics.drawable.Drawable
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.ScaleGestureDetector
import android.view.ViewConfiguration
import android.view.animation.DecelerateInterpolator
import androidx.appcompat.widget.AppCompatImageView
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

/**
 * An image that fits the view, zooms with pinch and double tap, pans while
 * zoomed, and can be dragged down to dismiss while it is not zoomed.
 *
 * While zoomed or pinching it keeps the parent (the pager) from taking the
 * gesture; otherwise horizontal drags go to the pager.
 */
class ZoomImageView(context: Context) : AppCompatImageView(context) {

    var onTap: (() -> Unit)? = null
    /** Drag-to-dismiss progress, 0 (at rest) to 1. */
    var onDismissProgress: ((Float) -> Unit)? = null
    var onDismiss: (() -> Unit)? = null
    /** When false the view only reports taps and the dismiss drag (video pages). */
    var zoomEnabled = true
    /** Called once each time the picture goes from fitted to clearly zoomed in. */
    var onZoomedIn: (() -> Unit)? = null
    private var zoomedInReported = false

    private var zoom = 1f
    private var tx = 0f
    private var ty = 0f
    private val drawMatrix = Matrix()
    private val slop = ViewConfiguration.get(context).scaledTouchSlop

    private var downX = 0f
    private var downY = 0f
    private var dismissing = false
    private var decided = false
    private var animator: ValueAnimator? = null

    val isZoomed: Boolean get() = zoom > 1.01f

    init {
        scaleType = ScaleType.MATRIX
    }

    private val scaleDetector = ScaleGestureDetector(context, object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
        override fun onScaleBegin(detector: ScaleGestureDetector): Boolean {
            if (!zoomEnabled) return false
            parent?.requestDisallowInterceptTouchEvent(true)
            return true
        }

        override fun onScale(detector: ScaleGestureDetector): Boolean {
            zoomAround(zoom * detector.scaleFactor, detector.focusX, detector.focusY, MIN_PINCH, MAX_ZOOM)
            return true
        }

        override fun onScaleEnd(detector: ScaleGestureDetector) {
            if (zoom < 1f) animateTo(1f, width / 2f, height / 2f)
        }
    })

    private val gestureDetector = GestureDetector(context, object : GestureDetector.SimpleOnGestureListener() {
        override fun onDown(e: MotionEvent): Boolean = true

        override fun onSingleTapConfirmed(e: MotionEvent): Boolean {
            onTap?.invoke()
            return true
        }

        override fun onDoubleTap(e: MotionEvent): Boolean {
            if (!zoomEnabled) return false
            if (isZoomed) animateTo(1f, e.x, e.y) else animateTo(DOUBLE_TAP_ZOOM, e.x, e.y)
            return true
        }

        override fun onScroll(e1: MotionEvent?, e2: MotionEvent, dx: Float, dy: Float): Boolean {
            if (!isZoomed || scaleDetector.isInProgress) return false
            tx -= dx
            ty -= dy
            applyMatrix()
            return true
        }
    })

    override fun onTouchEvent(event: MotionEvent): Boolean {
        scaleDetector.onTouchEvent(event)
        gestureDetector.onTouchEvent(event)

        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                animator?.cancel()
                downX = event.rawX
                downY = event.rawY
                dismissing = false
                decided = false
                if (isZoomed) parent?.requestDisallowInterceptTouchEvent(true)
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                // A second finger ends any dismiss drag in progress.
                if (dismissing) settleDismiss(false)
                decided = true
            }
            MotionEvent.ACTION_MOVE -> {
                if (isZoomed || event.pointerCount > 1) return true
                val dx = event.rawX - downX
                val dy = event.rawY - downY
                if (!decided && (abs(dx) > slop || abs(dy) > slop)) {
                    decided = true
                    dismissing = dy > 0 && abs(dy) > abs(dx) * 1.5f
                    if (dismissing) parent?.requestDisallowInterceptTouchEvent(true)
                }
                if (dismissing) {
                    val progress = min(1f, max(0f, dy) / (height * 0.6f))
                    translationX = dx
                    translationY = max(0f, dy)
                    val s = 1f - progress * 0.35f
                    scaleX = s
                    scaleY = s
                    onDismissProgress?.invoke(progress)
                }
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                if (dismissing) {
                    val dy = event.rawY - downY
                    settleDismiss(event.actionMasked == MotionEvent.ACTION_UP && dy > height * 0.16f)
                }
                parent?.requestDisallowInterceptTouchEvent(false)
            }
        }
        return true
    }

    private fun settleDismiss(dismiss: Boolean) {
        dismissing = false
        if (dismiss) {
            onDismiss?.invoke()
            return
        }
        animate().translationX(0f).translationY(0f).scaleX(1f).scaleY(1f).setDuration(180).start()
        onDismissProgress?.invoke(0f)
    }

    /** Back to the fitted, untransformed state (when the page scrolls away). */
    fun reset() {
        animator?.cancel()
        zoomedInReported = false
        zoom = 1f
        tx = 0f
        ty = 0f
        translationX = 0f
        translationY = 0f
        scaleX = 1f
        scaleY = 1f
        applyMatrix()
    }

    override fun setImageDrawable(drawable: Drawable?) {
        super.setImageDrawable(drawable)
        applyMatrix()
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        applyMatrix()
    }

    private fun zoomAround(target: Float, fx: Float, fy: Float, lo: Float, hi: Float) {
        val next = max(lo, min(hi, target))
        val k = next / zoom
        // Keep the image point under the focus where it is.
        val cx = width / 2f + tx
        val cy = height / 2f + ty
        tx = fx - width / 2f - (fx - cx) * k
        ty = fy - height / 2f - (fy - cy) * k
        zoom = next
        if (zoom > 1.3f && !zoomedInReported) {
            zoomedInReported = true
            onZoomedIn?.invoke()
        }
        applyMatrix()
    }

    private fun animateTo(target: Float, fx: Float, fy: Float) {
        animator?.cancel()
        val from = zoom
        animator = ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 220
            interpolator = DecelerateInterpolator()
            addUpdateListener {
                val t = it.animatedValue as Float
                zoomAround(from + (target - from) * t, fx, fy, MIN_PINCH, MAX_ZOOM)
            }
            start()
        }
    }

    private fun applyMatrix() {
        val d = drawable ?: return
        val dw = d.intrinsicWidth.toFloat()
        val dh = d.intrinsicHeight.toFloat()
        if (dw <= 0 || dh <= 0 || width == 0 || height == 0) return

        val scale = min(width / dw, height / dh) * zoom
        val shownW = dw * scale
        val shownH = dh * scale
        // The image may not leave the view further than its own overflow.
        val maxX = max(0f, (shownW - width) / 2f)
        val maxY = max(0f, (shownH - height) / 2f)
        tx = max(-maxX, min(maxX, tx))
        ty = max(-maxY, min(maxY, ty))

        drawMatrix.reset()
        drawMatrix.postScale(scale, scale)
        drawMatrix.postTranslate((width - shownW) / 2f + tx, (height - shownH) / 2f + ty)
        imageMatrix = drawMatrix
    }

    companion object {
        private const val MIN_PINCH = 0.8f
        private const val MAX_ZOOM = 5f
        private const val DOUBLE_TAP_ZOOM = 2.5f
    }
}
