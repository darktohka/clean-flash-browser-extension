package
{
    import flash.display.Sprite;
    import flash.display.Shape;
    import flash.text.TextField;

    /**
     * A minimal loadable SWF used as a target for Loader tests.
     * Exposes a public property and draws a colored rectangle so
     * we can verify it loaded correctly.
     */
    [SWF(width="200", height="100", backgroundColor="#336699", frameRate="30")]
    public class LoadableChild extends Sprite
    {
        public var childIdent:String = "LoadableChild-v1";
        public var loadedOk:Boolean = true;

        public function LoadableChild()
        {
            var rect:Shape = new Shape();
            rect.graphics.beginFill(0x336699);
            rect.graphics.drawRect(0, 0, 200, 100);
            rect.graphics.endFill();
            addChild(rect);

            var tf:TextField = new TextField();
            tf.text = "Child SWF";
            tf.width = 200;
            tf.textColor = 0xFFFFFF;
            tf.x = 10; tf.y = 10;
            addChild(tf);
        }
    }
}
