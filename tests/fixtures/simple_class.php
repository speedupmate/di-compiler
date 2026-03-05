<?php
namespace Magento\Framework\App;

class Application
{
    public function __construct(
        \Magento\Framework\AppInterface $app,
        \Magento\Framework\App\State $state
    ) {
        $this->app = $app;
    }

    public function run(): void
    {
        $this->app->launch();
    }
}
